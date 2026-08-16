import SwiftUI
import UIKit
import CoreImage.CIFilterBuiltins

private func dataDir() -> String {
    let u = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        .appendingPathComponent("nightfall", isDirectory: true)
    try? FileManager.default.createDirectory(at: u, withIntermediateDirectories: true)
    return u.path
}

private enum Tab: Hashable { case wallet, activity, settings }
private enum Sheet: Identifiable {
    case receive, send, seed, viewKey
    var id: String {
        switch self {
        case .receive: return "receive"
        case .send: return "send"
        case .seed: return "seed"
        case .viewKey: return "view"
        }
    }
}

private let accent = Color(red: 0.76, green: 0.31, blue: 0.88)
private let accent2 = Color(red: 0.61, green: 0.43, blue: 1.0)
private let dim = Color(red: 0.60, green: 0.57, blue: 0.70)
private let card = Color(red: 0.08, green: 0.06, blue: 0.13)
private let bg = Color(red: 0.027, green: 0.024, blue: 0.059)
private let ok = Color(red: 0.29, green: 0.87, blue: 0.50)

struct ContentView: View {
    @State private var wallet: MobileWallet?
    @State private var phrase: String?
    @State private var confirmed = false
    @State private var err: String?

    var body: some View {
        NavigationStack {
            ZStack {
                bg.ignoresSafeArea()
                if let w = wallet, confirmed {
                    HomeView(wallet: w, onWiped: {
                        wallet = nil
                        phrase = nil
                        confirmed = false
                    })
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
        DispatchQueue.global(qos: .userInitiated).async {
            let tip = (try? nodeTip(node: defaultNode())) ?? 0
            DispatchQueue.main.async {
                do {
                    let w = try MobileWallet.create(datadir: dataDir(), network: "mainnet", birthHeight: tip)
                    phrase = try w.recoveryPhrase()
                    wallet = w
                } catch { err = error.localizedDescription }
            }
        }
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
                Text("NIGHTFALLCOIN")
                    .font(.caption.weight(.semibold))
                    .tracking(3)
                    .foregroundStyle(accent2)
                    .padding(.top, 24)
                Text("Wallet").font(.largeTitle.bold())
                Text("Money that refuses to snitch.").foregroundStyle(dim)
                Text(privacyWarning()).font(.footnote).foregroundStyle(dim)
                Button("Create a wallet", action: onCreate)
                    .buttonStyle(.borderedProminent)
                    .tint(accent)
                Button("I have 24 words") { showRestore.toggle() }
                    .foregroundStyle(dim)
                if showRestore {
                    TextField("Recovery phrase", text: $words, axis: .vertical)
                        .textFieldStyle(.roundedBorder)
                    TextField("Birth height (0 if unsure)", text: $height)
                        .textFieldStyle(.roundedBorder)
                    Text("Too high silently misses coins. Too low only costs time.")
                        .font(.caption).foregroundStyle(dim)
                    Button("Restore") { onRestore(words, UInt64(height) ?? 0) }
                        .buttonStyle(.borderedProminent)
                        .tint(accent)
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
                Text("On paper. Not a photo, not the cloud.").foregroundStyle(dim)
                Text(phrase).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                Text("Type word 4 and word 18.").foregroundStyle(dim)
                TextField("Word 4", text: $a).textFieldStyle(.roundedBorder)
                TextField("Word 18", text: $b).textFieldStyle(.roundedBorder)
                let ok = words.dropFirst(3).first?.lowercased() == a.trimmingCharacters(in: .whitespaces).lowercased()
                    && words.dropFirst(17).first?.lowercased() == b.trimmingCharacters(in: .whitespaces).lowercased()
                Button("I have the words", action: onDone)
                    .disabled(!ok)
                    .buttonStyle(.borderedProminent)
                    .tint(accent)
                if let err { Text(err).foregroundStyle(.red) }
            }.padding(24)
        }
    }
}

struct HomeView: View {
    let wallet: MobileWallet
    var onWiped: () -> Void
    @AppStorage("nf.node") private var node = defaultNode()
    @AppStorage("nf.hide") private var hide = false
    @State private var address = ""
    @State private var bal: BalanceView?
    @State private var hist: [HistoryView] = []
    @State private var status = "syncing…"
    @State private var err: String?
    @State private var tab: Tab = .wallet
    @State private var sheet: Sheet?
    @State private var to = ""
    @State private var amt = ""
    @State private var memo = ""

    var body: some View {
        TabView(selection: $tab) {
            walletTab.tag(Tab.wallet).tabItem { Label("Wallet", systemImage: "wallet.pass") }
            activityTab.tag(Tab.activity).tabItem { Label("Activity", systemImage: "list.bullet") }
            settingsTab.tag(Tab.settings).tabItem { Label("Settings", systemImage: "gearshape") }
        }
        .tint(accent2)
        .sheet(item: $sheet) { s in
            NavigationStack {
                switch s {
                case .receive: receiveSheet
                case .send: sendSheet
                case .seed: secretSheet(title: "Recovery phrase", warn: "On paper. These 24 words are the wallet.", value: (try? wallet.recoveryPhrase()) ?? "", gated: true)
                case .viewKey: secretSheet(title: "View key", warn: "Reads amounts and memos. Cannot spend.", value: (try? wallet.viewKey()) ?? "", gated: false)
                }
            }
            .presentationDetents([.large])
        }
        .onAppear {
            address = (try? wallet.address()) ?? ""
            refresh()
        }
    }

    var walletTab: some View {
        ScrollView {
            VStack(spacing: 0) {
                ZStack {
                    LinearGradient(colors: [Color(red: 0.16, green: 0.09, blue: 0.35), bg], startPoint: .top, endPoint: .bottom)
                        .frame(height: 188)
                    VStack {
                        HStack {
                            Button("☰") { tab = .settings }.foregroundStyle(.white)
                            Spacer()
                            Button { refresh() } label: { Image(systemName: "arrow.clockwise") }.foregroundStyle(.white)
                        }.padding(.horizontal, 20).padding(.top, 8)
                        Text("NIGHTFALLCOIN")
                            .font(.caption.weight(.semibold))
                            .tracking(4)
                            .foregroundStyle(Color(red: 0.91, green: 0.86, blue: 1))
                            .padding(.top, 28)
                    }
                }
                VStack(alignment: .leading, spacing: 14) {
                    balanceCard
                    networkCard
                    HStack {
                        Text("Recent").font(.headline)
                        Spacer()
                        Button("See all") { tab = .activity }.font(.caption).foregroundStyle(dim)
                    }
                    txList(Array(hist.prefix(5)))
                }
                .padding(.horizontal, 14)
                .offset(y: -40)
                .padding(.bottom, 48)
            }
        }
        .background(bg)
    }

    var balanceCard: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Button {
                    hide.toggle()
                } label: {
                    HStack(spacing: 6) {
                        Text("Total balance").foregroundStyle(dim).font(.caption)
                        Image(systemName: hide ? "eye.slash" : "eye").font(.caption).foregroundStyle(dim)
                    }
                }
                Spacer()
                HStack(spacing: 6) {
                    Circle().fill(ok).frame(width: 7, height: 7)
                    Text("Nightfall").font(.caption2)
                }
                .padding(.horizontal, 9).padding(.vertical, 4)
                .background(Color.white.opacity(0.08), in: Capsule())
            }
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(hide ? "••••••" : pretty(bal?.total))
                    .font(.system(size: 32, weight: .bold, design: .monospaced))
                Text("NIGHT").font(.title3.weight(.bold)).foregroundStyle(accent2)
            }
            Text("spendable \(hide ? "••••" : pretty(bal?.available))  ·  unlocking \(hide ? "••••" : pretty(bal?.immature))")
                .font(.footnote).foregroundStyle(dim)
            HStack(spacing: 10) {
                Button { sheet = .receive } label: {
                    Label("Receive", systemImage: "arrow.down.left")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent).tint(accent)
                Button { sheet = .send } label: {
                    Label("Send", systemImage: "arrow.up.right")
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderedProminent).tint(Color(red: 0.24, green: 0.16, blue: 0.44))
            }
            .padding(.top, 6)
        }
        .padding(16)
        .background(Color(red: 0.08, green: 0.06, blue: 0.13).opacity(0.94), in: RoundedRectangle(cornerRadius: 22))
        .overlay(RoundedRectangle(cornerRadius: 22).stroke(Color.purple.opacity(0.2)))
    }

    var networkCard: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Network").font(.headline)
            row("Tip", bal.map { String($0.tipHeight) } ?? "—")
            row("Scanned to", bal.map { String($0.scannedTo) } ?? "—")
            row("Fee", "0.001 NIGHT · burned while subsidy lasts")
            Text(status).font(.caption).foregroundStyle(dim)
            if let err { Text(err).font(.caption).foregroundStyle(.red) }
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(card, in: RoundedRectangle(cornerRadius: 18))
    }

    var activityTab: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text("Activity").font(.caption).foregroundStyle(dim)
                Text("Movements").font(.largeTitle.bold())
                Text("Heights come from the node this phone trusts.").font(.footnote).foregroundStyle(dim)
                txList(hist)
            }.padding(20)
        }.background(bg)
    }

    var settingsTab: some View {
        let info = try? wallet.info()
        return ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text("Settings").font(.caption).foregroundStyle(dim)
                Text("Wallet").font(.largeTitle.bold())
                settingBtn("Recovery phrase", "24 words. Anyone who sees them can spend.") { sheet = .seed }
                settingBtn("View key", "Reads amounts and memos. Cannot spend.") { sheet = .viewKey }
                settingBtn(
                    "Rescan from birth height",
                    "Birth \(info?.birthHeight ?? 0) · scanned to \(info?.scannedTo ?? bal?.scannedTo ?? 0) · \(info?.outputs ?? 0) unspent"
                ) {
                    DispatchQueue.global(qos: .userInitiated).async {
                        _ = try? wallet.resetScan()
                        DispatchQueue.main.async { refresh() }
                    }
                }
                Text("Trusted node").font(.caption).foregroundStyle(dim).padding(.top, 8)
                TextField("node", text: $node)
                    .textFieldStyle(.roundedBorder)
                    .textInputAutocapitalization(.never)
                Text("This phone believes this node. Tip \(bal?.tipHeight ?? 0).").font(.caption).foregroundStyle(dim)
                Text(privacyWarning()).font(.caption2).foregroundStyle(dim).padding(.vertical, 8)
                Button("Remove wallet from this phone", role: .destructive) {
                    _ = try? wipeWallet(datadir: dataDir())
                    onWiped()
                }
                Text("0.7.0 · protocol v8 · not 100% anonymous").font(.caption2).foregroundStyle(dim).padding(.top, 12)
            }.padding(20)
        }.background(bg)
    }

    var receiveSheet: some View {
        ScrollView {
            VStack(spacing: 16) {
                Text("Share this address. Receiving works while this device is off.")
                    .font(.footnote).foregroundStyle(dim)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let img = qrImage(address) {
                    Image(uiImage: img)
                        .interpolation(.none)
                        .resizable()
                        .scaledToFit()
                        .frame(width: 240, height: 240)
                        .padding(14)
                        .background(Color(red: 0.96, green: 0.94, blue: 1), in: RoundedRectangle(cornerRadius: 18))
                }
                Text(address)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding()
                    .background(card, in: RoundedRectangle(cornerRadius: 16))
                Button("Copy address") { UIPasteboard.general.string = address }
                    .buttonStyle(.borderedProminent).tint(accent)
            }.padding(20)
        }
        .navigationTitle("Receive")
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { sheet = nil } } }
    }

    var sendSheet: some View {
        Form {
            Section {
                Text("Wrong address = gone forever. Fee 0.001 NIGHT, burned while blocks still pay a subsidy.")
                    .font(.footnote).foregroundStyle(dim)
                TextField("nf1…", text: $to)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("Amount", text: $amt).keyboardType(.decimalPad)
                TextField("Memo (optional)", text: $memo)
            }
            Section {
                Button("Send") {
                    status = "sending…"
                    let dest = to.trimmingCharacters(in: .whitespaces)
                    let amount = amt
                    let note = memo
                    sheet = nil
                    DispatchQueue.global(qos: .userInitiated).async {
                        do {
                            let id = try wallet.send(node: node, to: dest, amount: amount, memo: note)
                            DispatchQueue.main.async {
                                status = "sent \(id)"
                                to = ""; amt = ""; memo = ""
                                refresh()
                            }
                        } catch {
                            DispatchQueue.main.async { err = error.localizedDescription }
                        }
                    }
                }
                .disabled(!to.hasPrefix("nf1") || amt.isEmpty)
            }
        }
        .navigationTitle("Send")
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close") { sheet = nil } } }
    }

    func secretSheet(title: String, warn: String, value: String, gated: Bool) -> some View {
        SecretView(title: title, warn: warn, value: value, gated: gated) { sheet = nil }
    }

    func txList(_ rows: [HistoryView]) -> some View {
        VStack(spacing: 0) {
            if rows.isEmpty {
                Text("No movements yet.").foregroundStyle(dim).padding()
            }
            ForEach(Array(rows.enumerated()), id: \.offset) { _, e in
                HStack {
                    ZStack {
                        Circle().fill(Color(red: 0.11, green: 0.09, blue: 0.19)).frame(width: 38, height: 38)
                        Text((e.direction == "Received" || e.direction == "Mined") ? "↓" : "↑")
                    }
                    VStack(alignment: .leading) {
                        Text(e.direction).font(.subheadline.weight(.semibold))
                        Text(e.pending ? "pending" : (e.height.map { "#\($0)" } ?? "") + (e.memo.isEmpty ? "" : " · \(e.memo)"))
                            .font(.caption).foregroundStyle(dim).lineLimit(1)
                    }
                    Spacer()
                    let incoming = e.direction == "Received" || e.direction == "Mined"
                    Text((incoming ? "+" : "−") + pretty(e.amount) + " NIGHT")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(incoming ? ok : .primary)
                }
                .padding(.vertical, 8)
            }
        }
        .padding(14)
        .background(card, in: RoundedRectangle(cornerRadius: 18))
    }

    func settingBtn(_ title: String, _ sub: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(alignment: .leading, spacing: 3) {
                Text(title).foregroundStyle(.primary).font(.body.weight(.semibold))
                Text(sub).foregroundStyle(dim).font(.caption).multilineTextAlignment(.leading)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(14)
            .background(card, in: RoundedRectangle(cornerRadius: 14))
        }
    }

    func row(_ k: String, _ v: String) -> some View {
        HStack {
            Text(k).foregroundStyle(dim)
            Spacer()
            Text(v)
        }.font(.footnote)
    }

    func refresh() {
        status = "syncing…"
        err = nil
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
                DispatchQueue.main.async { err = error.localizedDescription; status = "sync failed" }
            }
        }
    }
}

struct SecretView: View {
    let title: String
    let warn: String
    let value: String
    let gated: Bool
    var onClose: () -> Void
    @State private var show = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(warn).font(.footnote).foregroundStyle(dim)
                if gated && !show {
                    Button("Show") { show = true }
                        .buttonStyle(.borderedProminent).tint(accent)
                } else {
                    Text(value)
                        .font(.system(.body, design: .monospaced))
                        .textSelection(.enabled)
                        .padding()
                        .background(card, in: RoundedRectangle(cornerRadius: 16))
                    Button("Copy") { UIPasteboard.general.string = value }
                        .buttonStyle(.borderedProminent).tint(accent)
                }
            }.padding(20)
        }
        .navigationTitle(title)
        .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Close", action: onClose) } }
        .onAppear { show = !gated }
    }
}

private func pretty(_ s: String?) -> String {
    guard let s, !s.isEmpty else { return "—" }
    let parts = s.split(separator: ".", maxSplits: 1, omittingEmptySubsequences: false)
    let whole = String(parts[0])
    var grouped = ""
    for (i, ch) in whole.reversed().enumerated() {
        if i > 0 && i % 3 == 0 { grouped.append(",") }
        grouped.append(ch)
    }
    let frac = parts.count > 1 ? String(parts[1]).trimmingCharacters(in: CharacterSet(charactersIn: "0")) : ""
    return frac.isEmpty ? "\(String(grouped.reversed())).00" : "\(String(grouped.reversed())).\(frac)"
}

private func qrImage(_ text: String) -> UIImage? {
    let f = CIFilter.qrCodeGenerator()
    f.message = Data(text.utf8)
    f.correctionLevel = "M"
    guard let out = f.outputImage else { return nil }
    let scaled = out.transformed(by: CGAffineTransform(scaleX: 10, y: 10))
    let ctx = CIContext()
    guard let cg = ctx.createCGImage(scaled, from: scaled.extent) else { return nil }
    return UIImage(cgImage: cg)
}
