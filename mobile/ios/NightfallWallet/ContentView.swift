import SwiftUI
import UIKit

private func dataDir() -> String {
    let u = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("nightfall", isDirectory: true)
    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
    return u.path
}

struct ContentView: View {
    @State private var wallet: MobileWallet?
    @State private var phrase: String?
    @State private var confirmed = false
    @State private var err: String?

    var body: some View {
        NavigationStack {
            ZStack {
                Color(red: 0.05, green: 0.04, blue: 0.09).ignoresSafeArea()
                if let w = wallet, confirmed {
                    HomeView(wallet: w)
                } else if let p = phrase, !confirmed {
                    BackupView(phrase: p, err: err) { confirmed = true }
                } else {
                    OnboardView(err: err, onCreate: create, onRestore: restore)
                }
            }
        }
        .onAppear {
            if walletExists(datadir: dataDir()) {
                do {
                    wallet = try MobileWallet.open(datadir: dataDir(), network: "mainnet")
                    confirmed = true
                } catch {
                    err = error.localizedDescription
                }
            }
        }
    }

    func create() {
        err = nil
        do {
            let w = try MobileWallet.create(datadir: dataDir(), network: "mainnet", birthHeight: 0)
            phrase = try w.recoveryPhrase()
            wallet = w
        } catch { err = error.localizedDescription }
    }

    func restore(words: String, height: UInt64) {
        err = nil
        do {
            wallet = try MobileWallet.restore(datadir: dataDir(), network: "mainnet", phrase: words, birthHeight: height)
            confirmed = true
        } catch { err = error.localizedDescription }
    }
}

struct OnboardView: View {
    var err: String?
    var onCreate: () -> Void
    var onRestore: (String, UInt64) -> Void
    @State private var showRestore = false
    @State private var words = ""
    @State private var height = "0"

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("NIGHT").font(.largeTitle.bold()).foregroundStyle(Color(red: 0.72, green: 0.27, blue: 0.85))
                Text("Money that refuses to snitch.").foregroundStyle(.secondary)
                Text(privacyWarning()).font(.footnote).foregroundStyle(.secondary)
                Button("Create a wallet", action: onCreate).buttonStyle(.borderedProminent)
                Button("I have 24 words") { showRestore.toggle() }
                if showRestore {
                    TextField("Recovery phrase", text: $words, axis: .vertical).textFieldStyle(.roundedBorder)
                    TextField("Birth height (0 if unsure)", text: $height).textFieldStyle(.roundedBorder)
                    Text("Too high silently misses coins. Too low only costs time.")
                        .font(.caption).foregroundStyle(.secondary)
                    Button("Restore") { onRestore(words, UInt64(height) ?? 0) }
                }
                if let err { Text(err).foregroundStyle(.red) }
            }.padding(24)
        }
    }
}

struct BackupView: View {
    let phrase: String
    var err: String?
    var onDone: () -> Void
    @State private var a = ""
    @State private var b = ""

    var body: some View {
        let words = phrase.split(whereSeparator: \.isWhitespace).map(String.init)
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Write these 24 words down.").font(.title2.bold())
                Text("On paper. Not a photo, not the cloud.").foregroundStyle(.secondary)
                Text(phrase).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                Text("Type word 4 and word 18.").foregroundStyle(.secondary)
                TextField("Word 4", text: $a).textFieldStyle(.roundedBorder)
                TextField("Word 18", text: $b).textFieldStyle(.roundedBorder)
                let ok = words.dropFirst(3).first?.lowercased() == a.trimmingCharacters(in: .whitespaces).lowercased()
                    && words.dropFirst(17).first?.lowercased() == b.trimmingCharacters(in: .whitespaces).lowercased()
                Button("I have the words", action: onDone).disabled(!ok).buttonStyle(.borderedProminent)
                if let err { Text(err).foregroundStyle(.red) }
            }.padding(24)
        }
    }
}

struct HomeView: View {
    let wallet: MobileWallet
    @State private var node = defaultNode()
    @State private var address = ""
    @State private var bal: BalanceView?
    @State private var hist: [HistoryView] = []
    @State private var status = "tap Sync"
    @State private var to = ""
    @State private var amt = ""
    @State private var memo = ""

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("NIGHT").font(.headline).foregroundStyle(Color(red: 0.72, green: 0.27, blue: 0.85))
                Text(bal?.total ?? "—").font(.system(size: 32, weight: .medium, design: .monospaced))
                Text("spendable \(bal?.available ?? "—")  ·  unlocking \(bal?.immature ?? "—")")
                    .font(.footnote).foregroundStyle(.secondary)
                Text(status).font(.caption).foregroundStyle(.secondary)
                HStack {
                    Button("Sync", action: refresh)
                    Button("Copy address") { UIPasteboard.general.string = address }
                }
                Text("Your address").font(.caption).foregroundStyle(.secondary)
                Text(address).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                Divider()
                Text("Send").font(.headline)
                Text("Wrong address = gone forever. Fee 0.001 NIGHT.")
                    .font(.caption).foregroundStyle(.secondary)
                TextField("nf1…", text: $to).textFieldStyle(.roundedBorder).textInputAutocapitalization(.never)
                TextField("Amount", text: $amt).textFieldStyle(.roundedBorder).keyboardType(.decimalPad)
                TextField("Memo (optional)", text: $memo).textFieldStyle(.roundedBorder)
                Button("Send") {
                    status = "sending…"
                    DispatchQueue.global(qos: .userInitiated).async {
                        do {
                            let id = try wallet.send(node: node, to: to.trimmingCharacters(in: .whitespaces), amount: amt, memo: memo)
                            DispatchQueue.main.async { status = "sent \(id)"; refresh() }
                        } catch {
                            DispatchQueue.main.async { status = error.localizedDescription }
                        }
                    }
                }.buttonStyle(.borderedProminent)
                Divider()
                Text("Activity").font(.headline)
                ForEach(Array(hist.enumerated()), id: \.offset) { _, e in
                    VStack(alignment: .leading) {
                        Text("\(e.direction)  \(e.amount)\(e.pending ? "  (pending)" : (e.height.map { "  #\($0)" } ?? ""))")
                            .font(.system(.footnote, design: .monospaced))
                        if !e.memo.isEmpty { Text(e.memo).font(.caption).foregroundStyle(.secondary) }
                    }
                }
                Text("Node this phone trusts").font(.caption).foregroundStyle(.secondary)
                TextField("node", text: $node).textFieldStyle(.roundedBorder).textInputAutocapitalization(.never)
                Text(privacyWarning()).font(.caption2).foregroundStyle(.secondary).padding(.bottom, 24)
            }.padding(20)
        }
        .onAppear {
            address = (try? wallet.address()) ?? ""
            refresh()
        }
    }

    func refresh() {
        status = "syncing…"
        DispatchQueue.global(qos: .userInitiated).async {
            do {
                let n = try wallet.sync(node: node)
                let a = try wallet.address()
                let b = try wallet.balance(node: node)
                let h = try wallet.history()
                DispatchQueue.main.async {
                    address = a
                    bal = b
                    hist = h
                    status = n == 0 ? "up to date" : "found \(n) new output(s)"
                }
            } catch {
                DispatchQueue.main.async { status = error.localizedDescription }
            }
        }
    }
}
