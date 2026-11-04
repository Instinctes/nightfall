#!/usr/bin/env node
/* Nightfall Reliability Lab — real nodes, real processes, devnet, loopback only.
 *
 * Roadmap 2027, milestone 5. Written 30 September 2026.
 *
 * Starts several `nightfalld --network devnet` processes on 127.0.0.1 with
 * private temporary data directories and runs scenarios that unit tests
 * cannot: a fresh node syncing from a peer, a clean restart, a hard kill and
 * restart, and a partition in which two nodes build competing branches and
 * must converge on the one with more work when they meet again. Every check
 * reads the nodes' own answers; nothing is simulated.
 *
 * Never touches mainnet, the default data directories, or the network beyond
 * loopback: devnet has no seed list, outbound SOCKS is switched off, and every
 * node is started with explicit --datadir and loopback listeners.
 *
 *     cargo build -p nightfall-node --bin nightfalld -p nightfall-nodekit
 *     node scripts/lab/run-lab.mjs [--out output/lab]
 *
 * Writes <out>/<timestamp>/report.json and report.md. No keys, seeds or
 * addresses are included — only heights, hashes, timings and verdicts.
 */
import net from "node:net";
import { Transform } from "node:stream";
import { once } from "node:events";
import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, cpSync, existsSync, readdirSync } from "node:fs";
import { tmpdir, platform, arch } from "node:os";
import { join, resolve } from "node:path";

const root = resolve(new URL("../..", import.meta.url).pathname);
const binDir=resolve(process.env.NIGHTFALL_LAB_BIN_DIR || join(root,"target/debug"));
const bin = join(binDir, "nightfalld");
const kit = join(binDir, "nightfall-nodekit");
const outArg = process.argv.indexOf("--out");
const outBase = resolve(root, outArg > 0 ? process.argv[outArg + 1] : "output/lab");
for (const b of [bin, kit]) if (!existsSync(b)) { console.error(`missing ${b}; build it first`); process.exit(2); }

const work = mkdtempSync(join(tmpdir(), "nightfall-lab-"));
const BASE_PORT = 28000 + Math.floor(Math.random() * 500) * 10;
const sleep = ms => new Promise(r => setTimeout(r, ms));
const results = [];
const running = new Set();

function node(name, index) {
    const datadir = join(work, name);
    const p2p = BASE_PORT + index * 3, rpc = p2p + 1, light = p2p + 2;
    return { name, datadir, p2p, rpc, light, proc: null, log: join(work, `${name}.log`) };
}

function cli(n, ...args) {
    return execFileSync(bin, ["--network", "devnet", "--datadir", n.datadir, ...args],
        { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], timeout: 600_000 });
}
const mine = (n, blocks) => cli(n, "mine", "--blocks", String(blocks), "--miner-seed", join(work, `${n.name}.seed`));

async function start(n, connect = [], minerSeed = null) {
    const args = ["--network", "devnet", "--datadir", n.datadir, "run",
        "--listen", `127.0.0.1:${n.p2p}`, "--rpc-listen", `127.0.0.1:${n.rpc}`,
        "--mobile-listen", `127.0.0.1:${n.light}`, "--proxy", "off",
        ...connect.flatMap(c => ["--connect", `127.0.0.1:${c.p2p}`]),
        ...(minerSeed ? ["--mine", "--miner-seed", minerSeed] : [])];
    const { openSync } = await import("node:fs");
    const fd = openSync(n.log, "a");
    n.proc = spawn(bin, args, { stdio: ["ignore", fd, fd], env: { ...process.env, NIGHTFALL_PEERS_URL: "" } });
    running.add(n);
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
        try { const s = await status(n); if (s && !s.loading) return s; } catch { /* not up yet */ }
        await sleep(250);
    }
    throw new Error(`${n.name} did not come up`);
}
async function stop(n, signal = "SIGTERM") {
    if (!n.proc) return;
    const p = n.proc; n.proc = null; running.delete(n);
    const gone = new Promise(r => p.once("exit", r));
    p.kill(signal);
    await Promise.race([gone, sleep(10_000)]);
    if (p.exitCode === null && p.signalCode === null) { p.kill("SIGKILL"); await gone; }
}
async function status(n) {
    const r = await fetch(`http://127.0.0.1:${n.light}/status`, { signal: AbortSignal.timeout(3000) });
    return (await r.json()).result;
}
async function waitFor(what, pred, ms = 90_000) {
    const deadline = Date.now() + ms;
    let last;
    while (Date.now() < deadline) {
        try { last = await pred(); if (last?.ok) return last; } catch (e) { last = { error: String(e) }; }
        await sleep(500);
    }
    throw new Error(`timed out waiting for ${what}: ${JSON.stringify(last)}`);
}
function doctor(n) {
    try {
        return JSON.parse(execFileSync(kit, ["--network", "devnet", "--json", "doctor", "--datadir", n.datadir], { encoding: "utf8" }));
    } catch (e) {
        return JSON.parse(e.stdout);
    }
}
function copyChain(from, to) {
    mkdirSync(to.datadir, { recursive: true });
    for (const f of readdirSync(from.datadir)) {
        if (f === ".nightfall-lock" || f.endsWith(".seed")) continue;
        cpSync(join(from.datadir, f), join(to.datadir, f), { recursive: true });
    }
}

async function scenario(name, body) {
    const t0 = Date.now();
    process.stdout.write(`  ..   ${name}\r`);
    try {
        const evidence = await body();
        results.push({ name, verdict: "pass", ms: Date.now() - t0, evidence });
        console.log(`  ok   ${name}  (${((Date.now() - t0) / 1000).toFixed(1)} s)`);
    } catch (e) {
        results.push({ name, verdict: "fail", ms: Date.now() - t0, error: e.message });
        console.log(`  FAIL ${name}\n       ${e.message}`);
    } finally {
        for (const n of [...running]) await stop(n);
    }
}

const A = node("a", 0), B = node("b", 1), C = node("c", 2), D = node("interrupted-sync",3);
console.log(`Nightfall Reliability Lab — devnet, loopback ports ${BASE_PORT}+, work dir ${work}`);

await scenario("fresh node syncs from a peer to the same tip", async () => {
    cli(A, "init"); mine(A, 20);
    const sa = await start(A);
    await start(B, [A]);
    const got = await waitFor("B to reach A's tip", async () => {
        const sb = await status(B);
        return { ok: sb.tip === sa.tip, a: sa.tip_height, b: sb.tip_height, tip: sb.tip };
    });
    return { height: got.b, tip: got.tip, genesis: sa.genesis };
});

await scenario("clean restart keeps the chain and passes doctor", async () => {
    const before = await start(B);
    await stop(B);
    const d = doctor(B);
    const fails = d.findings.filter(f => f.level === "fail");
    if (fails.length) throw new Error(`doctor failed: ${JSON.stringify(fails)}`);
    const after = await start(B);
    if (after.tip !== before.tip) throw new Error(`tip changed across restart: ${before.tip} → ${after.tip}`);
    return { height: after.tip_height, tip: after.tip, doctor: d.findings.map(f => `${f.level}:${f.check}`) };
});

await scenario("hard kill (SIGKILL) and restart: lock recovers, chain intact", async () => {
    const before = await start(B);
    await stop(B, "SIGKILL");
    const d = doctor(B);
    const lock = d.findings.find(f => f.check === "lock");
    if (d.findings.some(f => f.level === "fail")) throw new Error(`doctor failed after kill: ${JSON.stringify(d.findings)}`);
    const after = await start(B);
    if (after.tip_height < before.tip_height) throw new Error(`lost blocks: ${before.tip_height} → ${after.tip_height}`);
    return { height: after.tip_height, lock_after_kill: lock?.detail ?? "(none)" };
});

await scenario("partition: competing branches converge on the one with more work", async () => {
    // Common history, then each side builds alone. B builds the longer branch.
    copyChain(A, C);
    mine(A, 2);
    mine(C, 5);
    const sa0 = await start(A), sc0 = await start(C);
    if (sa0.tip === sc0.tip) throw new Error("branches did not diverge");
    await stop(A); await stop(C);
    // Heal the partition: A connects to C.
    await start(C);
    await start(A, [C]);
    const got = await waitFor("A to reorganise onto C's branch", async () => {
        const [sa, sc] = await Promise.all([status(A), status(C)]);
        return { ok: sa.tip === sc.tip, a: sa.tip_height, c: sc.tip_height, tip: sc.tip, a_tip: sa.tip };
    }, 120_000);
    if (got.tip !== sc0.tip) throw new Error("converged on a tip other than the heavier branch");
    return { a_branch: { height: sa0.tip_height, tip: sa0.tip }, c_branch: { height: sc0.tip_height, tip: sc0.tip }, converged: { height: got.a, tip: got.tip } };
});

await scenario("a third node joining after the reorg sees only the winning branch", async () => {
    // B still holds the pre-partition chain; it must follow the network, not its memory.
    await start(C);
    await start(A, [C]);
    await start(B, [A]);
    const sc = await status(C);
    const got = await waitFor("B to follow", async () => {
        const sb = await status(B);
        return { ok: sb.tip === sc.tip, b: sb.tip_height, tip: sb.tip };
    });
    return { height: got.b, tip: got.tip };
});

await scenario("bounded restart soak: five cycles preserve the same verified branch", async () => {
    const evidence=[];
    for(let cycle=0;cycle<5;cycle++) {
        const source=await start(C);await start(B,[C]);
        const current=await waitFor("soak sync",async()=>{const s=await status(B);return {ok:s.tip===source.tip,height:s.tip_height,tip:s.tip};});
        await stop(B,cycle%2?"SIGKILL":"SIGTERM");const d=doctor(B);
        if(d.findings.some(f=>f.level==="fail"))throw new Error("Doctor failed in restart cycle "+cycle);
        evidence.push({cycle:cycle+1,signal:cycle%2?"SIGKILL":"SIGTERM",...current});await stop(C);
    }
    return {cycles:evidence,scope:"bounded regression, not a multi-day soak"};
});

// Bound the link throughput so interruption happens while response bytes are
// arriving, rather than claiming an idle kill proves active sync recovery.
await scenario("interrupt active sync through a throttled link, then recover exact tip", async () => {
    const source=await start(C); let bytes=0; const sockets=new Set();
    const proxy=net.createServer(client=>{
        const upstream=net.connect(C.p2p,"127.0.0.1");
        sockets.add(client);sockets.add(upstream);
        class Slow extends Transform {
            _transform(chunk,encoding,done) {
                let offset=0;
                const send=()=>{ if(this.destroyed){done();return;} const part=chunk.subarray(offset,offset+512);offset+=part.length;bytes+=part.length;this.push(part);if(offset<chunk.length)setTimeout(send,5);else done(); };
                setTimeout(send,5);
            }
        }
        client.pipe(upstream);upstream.pipe(new Slow()).pipe(client);
        for(const socket of [client,upstream]) {socket.on("error",()=>{client.destroy();upstream.destroy();});socket.on("close",()=>sockets.delete(socket));}
    });
    proxy.listen(0,"127.0.0.1");await once(proxy,"listening");
    let interrupted;
    try {
        await start(D,[{p2p:proxy.address().port}]);
        interrupted=await waitFor("active incomplete sync",async()=>{ const s=await status(D);return {ok:bytes>2048 && s.tip_height<source.tip_height,received_bytes:bytes,height:s.tip_height,target:source.tip_height};},15000);
        await stop(D,"SIGKILL");
    } finally {for(const s of sockets)s.destroy();await new Promise(resolve=>proxy.close(resolve));}
    await start(D,[C]);
    const recovered=await waitFor("interrupted node to recover",async()=>{const s=await status(D);return {ok:s.tip===source.tip,height:s.tip_height,tip:s.tip};});
    await stop(D);const d=doctor(D);if(d.findings.some(f=>f.level==="fail"))throw new Error("Doctor failed after sync interruption");
    return {interrupted,recovered,scope:"SIGKILL while peer response is arriving; exact disk-write instruction not instrumented"};
});

await scenario("solo Devnet miner reaches coinbase maturity without a peer", async () => {
    const solo=node("solo-mining",4);cli(solo,"init");
    const seed=join(work,"solo.seed");writeFileSync(seed,"7b".repeat(32),{mode:0o600});
    await start(solo,[],seed);
    const reached=await waitFor("solo miner maturity",async()=>{const s=await status(solo);return {ok:s.tip_height>=10, height:s.tip_height, maturity:10, live_peers:s.live_peers};},30000);
    if(reached.live_peers!==0)throw new Error("Solo mining test unexpectedly connected to a peer");
    return reached;
});

// Report.
const commit = (() => { try { return execFileSync("git", ["-C", root, "rev-parse", "--short", "HEAD"], { encoding: "utf8" }).trim(); } catch { return "unknown"; } })();
const dirty = (() => { try { return execFileSync("git", ["-C", root, "status", "--porcelain", "--untracked-files=no"], { encoding: "utf8" }).trim() !== ""; } catch { return null; } })();
const version = execFileSync(bin, ["--version"], { encoding: "utf8" }).trim();
const stamp = new Date().toISOString().replace(/[:.]/g, "-");
const out = join(outBase, stamp);
mkdirSync(out, { recursive: true });
const report = {
    kind: "nightfall-reliability-lab", when: new Date().toISOString(), commit, dirty_tree: dirty, binary: version,
    host: { platform: platform(), arch: arch() },
    network: "devnet", scope: "loopback only, private temp datadirs, no seeds, SOCKS off",
    scenarios: results,
    not_covered: [
        "different machines, operating systems or real network latency/loss",
        "mainnet-sized chains and long-running soak",
        "adversarial peers (malformed messages, eclipse attempts)",
        "hardware faults and deterministic kill inside an fsync/write instruction (not instrumented)",
        "cutting a live link: the partition is made by keeping nodes apart while they mine, then connecting them",
    ],
};
writeFileSync(join(out, "report.json"), JSON.stringify(report, null, 2) + "\n");
const md = [
    `# Reliability Lab report — ${report.when}`, "",
    `Commit \`${commit}\`${dirty ? " (uncommitted changes present)" : ""} · ${version} · ${platform()}/${arch()} · devnet, loopback only`, "",
    "| Scenario | Verdict | Time |", "|---|---|---|",
    ...results.map(r => `| ${r.name} | ${r.verdict === "pass" ? "pass" : "**FAIL**"} | ${(r.ms / 1000).toFixed(1)} s |`), "",
    "## Evidence", "",
    ...results.map(r => `- **${r.name}**: \`${JSON.stringify(r.evidence ?? { error: r.error })}\``), "",
    "## Not covered by this run", "", ...report.not_covered.map(x => `- ${x}`), "",
];
writeFileSync(join(out, "report.md"), md.join("\n"));
const failed = results.filter(r => r.verdict !== "pass").length;
console.log(`\n${results.length - failed}/${results.length} scenarios passed · report: ${out}`);
if (!failed) rmSync(work, { recursive: true, force: true });
else console.log(`logs kept in ${work}`);
process.exit(failed ? 1 : 0);
