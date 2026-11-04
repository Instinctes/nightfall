import { readFileSync, existsSync } from "node:fs";

// The version the website must describe: the one that has been *published*.
//
// Until October 2026 this was simply the workspace Cargo.toml. Then the next
// version (1.1.0) was developed in the workspace while 1.0.6 was the live
// release, and every website check — including the weekly chain archive —
// failed with "Release channel 1.0.6 does not match source 1.1.0", although the
// site was right. The published source is the public mirror in github/, so that
// is the authority when it exists; elsewhere (CI, a fresh clone) the workspace
// is the only tree there is, and it is used as before.
export function publishedVersion(root) {
    for (const file of ["github/Cargo.toml", "Cargo.toml"]) {
        if (!existsSync(root + file)) continue;
        const match = readFileSync(root + file, "utf8").match(/^version\s*=\s*"([^"]+)"/m);
        if (match) return match[1];
    }
    return undefined;
}

// A development preview must not silently replace the stable download channel.
// Both channels are explicit, and the active one must match the source version.
export function releaseChannels(root, version) {
    const channels = JSON.parse(readFileSync(root + "website/public/releases.json", "utf8"));
    if (!/^\d+\.\d+\.\d+$/.test(channels.stable)) throw new Error("Invalid stable release");
    if (channels.development && !/^\d+\.\d+\.\d+-dev\.\d+$/.test(channels.development)) {
        throw new Error("Invalid development release");
    }
    const active = version.includes("-") ? channels.development : channels.stable;
    if (active !== version) throw new Error(`Release channel ${active} does not match source ${version}`);
    return new Set([channels.stable, channels.development].filter(Boolean));
}

export const RELEASE_VERSION = String.raw`\d+\.\d+\.\d+(?:-dev\.\d+)?`;
