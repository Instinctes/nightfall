package org.nightfallcoin.wallet

import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.AccountBalanceWallet
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.NorthEast
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.SouthWest
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nightfall_mobile.BalanceView
import uniffi.nightfall_mobile.HistoryView
import uniffi.nightfall_mobile.MobileWallet
import uniffi.nightfall_mobile.defaultFee
import uniffi.nightfall_mobile.defaultNode
import uniffi.nightfall_mobile.nodeTip
import uniffi.nightfall_mobile.privacyWarning
import uniffi.nightfall_mobile.walletExists
import uniffi.nightfall_mobile.wipeWallet

private val Bg = Color(0xFF07060F)
private val Card = Color(0xFF141022)
private val Accent = Color(0xFFC14EE0)
private val Accent2 = Color(0xFF9B6DFF)
private val TextCol = Color(0xFFF4F0FF)
private val Dim = Color(0xFF9A92B3)
private val Ok = Color(0xFF4ADE80)
private val Bad = Color(0xFFFF6B8A)

private enum class Tab { Wallet, Activity, Settings }
private enum class Sheet { None, Receive, Send, Seed, ViewKey }

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val dir = filesDir.absolutePath
        val prefs = getSharedPreferences("nf", MODE_PRIVATE)
        setContent {
            MaterialTheme(
                colorScheme = darkColorScheme(
                    background = Bg,
                    surface = Card,
                    primary = Accent,
                    onPrimary = Color.White,
                    onBackground = TextCol,
                ),
            ) {
                Surface(Modifier.fillMaxSize().background(Bg), color = Bg) {
                    App(dir, prefs)
                }
            }
        }
    }
}

@Composable
private fun App(dir: String, prefs: android.content.SharedPreferences) {
    var wallet by remember { mutableStateOf<MobileWallet?>(null) }
    var phrase by remember { mutableStateOf<String?>(null) }
    var confirmed by remember { mutableStateOf(false) }
    var err by remember { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        if (walletExists(dir)) {
            runCatching { MobileWallet.open(dir, "mainnet") }
                .onSuccess { wallet = it; confirmed = true }
                .onFailure { err = it.message }
        }
    }

    when {
        wallet == null -> Onboard(
            err = err,
            onCreate = {
                err = null
                scope.launch {
                    runCatching {
                        val node = prefs.getString("node", defaultNode()) ?: defaultNode()
                        val tip = withContext(Dispatchers.IO) {
                            runCatching { nodeTip(node) }.getOrDefault(0uL)
                        }
                        val w = MobileWallet.create(dir, "mainnet", tip)
                        phrase = w.recoveryPhrase()
                        wallet = w
                    }.onFailure { err = it.message }
                }
            },
            onRestore = { words, height ->
                err = null
                runCatching {
                    wallet = MobileWallet.restore(dir, "mainnet", words, height.toULong())
                    confirmed = true
                }.onFailure { err = it.message }
            },
        )
        !confirmed && phrase != null -> Backup(
            phrase = phrase!!,
            err = err,
            onDone = { confirmed = true },
        )
        else -> Home(wallet!!, dir, prefs) {
            wallet = null
            phrase = null
            confirmed = false
        }
    }
}

@Composable
private fun Onboard(err: String?, onCreate: () -> Unit, onRestore: (String, Long) -> Unit) {
    var restore by remember { mutableStateOf(false) }
    var words by remember { mutableStateOf("") }
    var height by remember { mutableStateOf("0") }
    Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState())) {
        Text("NIGHTFALLCOIN", color = Accent2, letterSpacing = 4.sp, fontSize = 14.sp, modifier = Modifier.padding(top = 24.dp))
        Text("Wallet", color = TextCol, fontSize = 32.sp, fontWeight = FontWeight.Bold)
        Text("Money that refuses to snitch.", color = Dim, modifier = Modifier.padding(top = 6.dp, bottom = 16.dp))
        Text(privacyWarning(), color = Dim, fontSize = 13.sp)
        Spacer(Modifier.height(28.dp))
        Button(onClick = onCreate, modifier = Modifier.fillMaxWidth(), colors = ButtonDefaults.buttonColors(containerColor = Accent)) {
            Text("Create a wallet")
        }
        TextButton(onClick = { restore = !restore }, modifier = Modifier.fillMaxWidth()) { Text("I have 24 words", color = Dim) }
        if (restore) {
            OutlinedTextField(words, { words = it }, label = { Text("Recovery phrase") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(height, { height = it }, label = { Text("Birth height (0 if unsure)") }, modifier = Modifier.fillMaxWidth())
            Text("A number that is too high silently misses coins. Too low only costs time.", color = Dim, fontSize = 12.sp)
            Button(onClick = { onRestore(words, height.toLongOrNull() ?: 0L) }, modifier = Modifier.fillMaxWidth()) { Text("Restore") }
        }
        err?.let { Text(it, color = Bad, modifier = Modifier.padding(top = 16.dp)) }
    }
}

@Composable
private fun Backup(phrase: String, err: String?, onDone: () -> Unit) {
    val words = phrase.trim().split(Regex("\\s+"))
    var a by remember { mutableStateOf("") }
    var b by remember { mutableStateOf("") }
    var copied by remember { mutableStateOf(false) }
    val ctx = LocalContext.current
    Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState())) {
        Text("Write these 24 words down.", color = TextCol, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
        Text(
            "Paper first. A password manager is next-best. Not a photo, not chat. Anyone with these words can spend.",
            color = Dim,
            modifier = Modifier.padding(vertical = 8.dp),
        )
        WordGrid(words)
        OutlinedButton(
            onClick = {
                val cm = ctx.getSystemService(ClipboardManager::class.java)
                cm.setPrimaryClip(ClipData.newPlainText("night-seed", phrase))
                copied = true
            },
            modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
        ) { Text("Copy all 24 words") }
        if (copied) Text("Copied. Clear the clipboard when you have stored them.", color = Ok, fontSize = 13.sp, modifier = Modifier.padding(top = 6.dp))
        Text("Type word 4 and word 18.", color = Dim, modifier = Modifier.padding(top = 16.dp))
        OutlinedTextField(a, { a = it }, label = { Text("Word 4") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(b, { b = it }, label = { Text("Word 18") }, modifier = Modifier.fillMaxWidth())
        val ok = words.getOrNull(3).equals(a.trim(), true) && words.getOrNull(17).equals(b.trim(), true)
        Button(onClick = onDone, enabled = ok, modifier = Modifier.fillMaxWidth().padding(top = 16.dp)) { Text("I have the words") }
        err?.let { Text(it, color = Bad) }
    }
}

@Composable
private fun WordGrid(words: List<String>) {
    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        words.chunked(2).forEachIndexed { row, pair ->
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.fillMaxWidth()) {
                pair.forEachIndexed { col, w ->
                    val n = row * 2 + col + 1
                    Surface(shape = RoundedCornerShape(10.dp), color = Card, modifier = Modifier.weight(1f)) {
                        Row(Modifier.padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
                            Text("$n", color = Dim, fontSize = 11.sp, modifier = Modifier.width(22.dp))
                            Text(w, color = TextCol, fontFamily = FontFamily.Monospace, fontSize = 14.sp)
                        }
                    }
                }
                if (pair.size == 1) Spacer(Modifier.weight(1f))
            }
        }
    }
}

@Composable
private fun Home(
    wallet: MobileWallet,
    dir: String,
    prefs: android.content.SharedPreferences,
    onWiped: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val ctx = LocalContext.current
    var node by remember { mutableStateOf(prefs.getString("node", defaultNode()) ?: defaultNode()) }
    var address by remember { mutableStateOf("") }
    var bal by remember { mutableStateOf<BalanceView?>(null) }
    var hist by remember { mutableStateOf(listOf<HistoryView>()) }
    var status by remember { mutableStateOf("syncing…") }
    var err by remember { mutableStateOf<String?>(null) }
    var tab by remember { mutableStateOf(Tab.Wallet) }
    var sheet by remember { mutableStateOf(Sheet.None) }
    var hide by remember { mutableStateOf(prefs.getBoolean("hide", false)) }
    var to by remember { mutableStateOf("") }
    var amt by remember { mutableStateOf("") }
    var memo by remember { mutableStateOf("") }
    val fee = remember { runCatching { defaultFee() }.getOrDefault("0.00100000") }

    fun refresh() {
        scope.launch {
            status = "syncing…"
            err = null
            val r = withContext(Dispatchers.IO) {
                runCatching {
                    val found = wallet.sync(node)
                    address = wallet.address()
                    bal = wallet.balance(node)
                    hist = wallet.history()
                    found
                }
            }
            r.fold(
                { n -> status = if (n == 0u) "up to date" else "found $n new output(s)" },
                { e -> err = e.message ?: "sync failed"; status = "sync failed" },
            )
        }
    }

    LaunchedEffect(Unit) {
        address = runCatching { wallet.address() }.getOrDefault("")
        refresh()
    }

    fun copy(s: String) {
        val cm = ctx.getSystemService(ClipboardManager::class.java)
        cm.setPrimaryClip(ClipData.newPlainText("night", s))
        status = "copied"
    }

    Scaffold(
        containerColor = Bg,
        bottomBar = {
            if (sheet == Sheet.None) {
                NavigationBar(containerColor = Color(0xEE0A0812)) {
                    NavigationBarItem(
                        selected = tab == Tab.Wallet,
                        onClick = { tab = Tab.Wallet },
                        icon = { Icon(Icons.Outlined.AccountBalanceWallet, null) },
                        label = { Text("Wallet") },
                    )
                    NavigationBarItem(
                        selected = tab == Tab.Activity,
                        onClick = { tab = Tab.Activity },
                        icon = { Icon(Icons.Outlined.SouthWest, null) },
                        label = { Text("Activity") },
                    )
                    NavigationBarItem(
                        selected = tab == Tab.Settings,
                        onClick = { tab = Tab.Settings },
                        icon = { Icon(Icons.Outlined.Settings, null) },
                        label = { Text("Settings") },
                    )
                }
            }
        },
    ) { pad ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(pad)
                .verticalScroll(rememberScrollState()),
        ) {
            when {
                sheet == Sheet.Receive -> ReceivePane(address, ::copy) { sheet = Sheet.None }
                sheet == Sheet.Send -> SendPane(
                    to, amt, memo, fee, bal?.available,
                    onTo = { to = it }, onAmt = { amt = it }, onMemo = { memo = it },
                    onBack = { sheet = Sheet.None },
                    onSend = {
                        scope.launch {
                            status = "sending…"
                            sheet = Sheet.None
                            tab = Tab.Wallet
                            val r = withContext(Dispatchers.IO) {
                                runCatching { wallet.send(node, to.trim(), amt.trim(), memo) }
                            }
                            r.fold(
                                { status = "sent $it"; to = ""; amt = ""; memo = ""; refresh() },
                                { err = it.message ?: "send failed" },
                            )
                        }
                    },
                )
                sheet == Sheet.Seed -> SecretPane(
                    title = "Recovery phrase",
                    body = "On paper. Not a photo. These 24 words are the wallet.",
                    secret = runCatching { wallet.recoveryPhrase() }.getOrDefault(""),
                    onCopy = { copy(it) },
                    onBack = { sheet = Sheet.None },
                )
                sheet == Sheet.ViewKey -> SecretPane(
                    title = "View key",
                    body = "Reads amounts and memos. Cannot spend.",
                    secret = runCatching { wallet.viewKey() }.getOrDefault(""),
                    onCopy = { copy(it) },
                    onBack = { sheet = Sheet.None },
                    revealFirst = true,
                )
                tab == Tab.Activity -> ActivityPane(hist)
                tab == Tab.Settings -> SettingsPane(
                    wallet = wallet,
                    node = node,
                    onNode = {
                        node = it
                        prefs.edit().putString("node", it).apply()
                    },
                    bal = bal,
                    status = status,
                    onRescan = {
                        scope.launch {
                            withContext(Dispatchers.IO) { runCatching { wallet.resetScan() } }
                            refresh()
                        }
                    },
                    onSeed = { sheet = Sheet.Seed },
                    onView = { sheet = Sheet.ViewKey },
                    onWipe = {
                        runCatching { wipeWallet(dir) }
                        onWiped()
                    },
                )
                else -> WalletPane(
                    bal = bal,
                    hist = hist,
                    status = status,
                    err = err,
                    hide = hide,
                    onHide = {
                        hide = !hide
                        prefs.edit().putBoolean("hide", hide).apply()
                    },
                    onSync = { refresh() },
                    onReceive = { sheet = Sheet.Receive },
                    onSend = { sheet = Sheet.Send },
                    onAll = { tab = Tab.Activity },
                    onSettings = { tab = Tab.Settings },
                )
            }
        }
    }
}

@Composable
private fun WalletPane(
    bal: BalanceView?,
    hist: List<HistoryView>,
    status: String,
    err: String?,
    hide: Boolean,
    onHide: () -> Unit,
    onSync: () -> Unit,
    onReceive: () -> Unit,
    onSend: () -> Unit,
    onAll: () -> Unit,
    onSettings: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxWidth()
            .height(188.dp)
            .background(Brush.verticalGradient(listOf(Color(0xFF2A1858), Color(0xFF140C28), Bg))),
    ) {
        Row(
            Modifier.fillMaxWidth().padding(16.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            TextButton(onClick = onSettings) { Text("☰", color = TextCol) }
            IconButton(onClick = onSync) { Icon(Icons.Outlined.Sync, "Sync", tint = TextCol) }
        }
        Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
            Text("NIGHTFALLCOIN", color = Color(0xFFE8DCFF), letterSpacing = 5.sp, fontSize = 12.sp, fontWeight = FontWeight.SemiBold)
        }
    }
    Column(Modifier.offset(y = (-36).dp).padding(horizontal = 14.dp)) {
        Surface(shape = RoundedCornerShape(22.dp), color = Color(0xF2141022), tonalElevation = 4.dp) {
            Column(Modifier.padding(16.dp)) {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("Total balance", color = Dim, fontSize = 13.sp)
                        IconButton(onClick = onHide, modifier = Modifier.size(28.dp)) {
                            Icon(if (hide) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility, null, tint = Dim)
                        }
                    }
                    Surface(shape = RoundedCornerShape(99.dp), color = Color(0x22FFFFFF)) {
                        Row(Modifier.padding(horizontal = 9.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(Modifier.size(7.dp).clip(CircleShape).background(Ok))
                            Spacer(Modifier.width(6.dp))
                            Text("Nightfall", color = TextCol, fontSize = 11.sp)
                        }
                    }
                }
                Row(verticalAlignment = Alignment.Bottom) {
                    Text(
                        if (hide) "••••••" else pretty(bal?.total),
                        color = TextCol,
                        fontSize = 32.sp,
                        fontWeight = FontWeight.Bold,
                        fontFamily = FontFamily.Monospace,
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("NIGHT", color = Accent2, fontWeight = FontWeight.Bold, modifier = Modifier.padding(bottom = 4.dp))
                }
                Text(
                    "spendable ${if (hide) "••••" else pretty(bal?.available)}  ·  unlocking ${if (hide) "••••" else pretty(bal?.immature)}",
                    color = Dim,
                    fontSize = 13.sp,
                    modifier = Modifier.padding(top = 4.dp, bottom = 14.dp),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    Button(onClick = onReceive, modifier = Modifier.weight(1f), colors = ButtonDefaults.buttonColors(containerColor = Accent), shape = RoundedCornerShape(99.dp)) {
                        Icon(Icons.Outlined.SouthWest, null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Receive")
                    }
                    Button(onClick = onSend, modifier = Modifier.weight(1f), colors = ButtonDefaults.buttonColors(containerColor = Color(0xFF3D2870)), shape = RoundedCornerShape(99.dp)) {
                        Icon(Icons.Outlined.NorthEast, null, modifier = Modifier.size(16.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Send")
                    }
                }
            }
        }
        Spacer(Modifier.height(14.dp))
        Surface(shape = RoundedCornerShape(18.dp), color = Card, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(16.dp)) {
                Text("Network", color = TextCol, fontWeight = FontWeight.SemiBold)
                Kv("Tip", bal?.tipHeight?.toString() ?: "—")
                Kv("Scanned to", bal?.scannedTo?.toString() ?: "—")
                Kv("Fee", "0.001 NIGHT · burned while subsidy lasts")
                Text(status, color = Dim, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp))
                err?.let { Text(it, color = Bad, fontSize = 12.sp) }
            }
        }
        Spacer(Modifier.height(14.dp))
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text("Recent", color = TextCol, fontWeight = FontWeight.SemiBold)
            TextButton(onClick = onAll) { Text("See all", color = Dim, fontSize = 13.sp) }
        }
        Surface(shape = RoundedCornerShape(18.dp), color = Card, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(horizontal = 14.dp, vertical = 6.dp)) {
                if (hist.isEmpty()) Text("No movements yet.", color = Dim, modifier = Modifier.padding(12.dp))
                hist.take(5).forEach { TxRow(it) }
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun ActivityPane(hist: List<HistoryView>) {
    Column(Modifier.padding(20.dp)) {
        Text("Activity", color = Dim, fontSize = 13.sp)
        Text("Movements", color = TextCol, fontSize = 28.sp, fontWeight = FontWeight.Bold)
        Text("Heights come from the node this phone trusts.", color = Dim, fontSize = 13.sp, modifier = Modifier.padding(bottom = 12.dp))
        Surface(shape = RoundedCornerShape(18.dp), color = Card, modifier = Modifier.fillMaxWidth()) {
            Column(Modifier.padding(horizontal = 14.dp, vertical = 6.dp)) {
                if (hist.isEmpty()) Text("No movements yet.", color = Dim, modifier = Modifier.padding(12.dp))
                hist.forEach { TxRow(it) }
            }
        }
    }
}

@Composable
private fun SettingsPane(
    wallet: MobileWallet,
    node: String,
    onNode: (String) -> Unit,
    bal: BalanceView?,
    status: String,
    onRescan: () -> Unit,
    onSeed: () -> Unit,
    onView: () -> Unit,
    onWipe: () -> Unit,
) {
    val info = remember { runCatching { wallet.info() }.getOrNull() }
    var pendingWipe by remember { mutableStateOf(false) }
    Column(Modifier.padding(20.dp)) {
        Text("Settings", color = Dim, fontSize = 13.sp)
        Text("Wallet", color = TextCol, fontSize = 28.sp, fontWeight = FontWeight.Bold, modifier = Modifier.padding(bottom = 12.dp))
        SettingBtn("Recovery phrase", "24 words. Anyone who sees them can spend.", onSeed)
        SettingBtn("View key", "Reads amounts and memos. Cannot spend.", onView)
        SettingBtn(
            "Rescan from birth height",
            "Birth ${info?.birthHeight ?: "—"} · scanned to ${info?.scannedTo ?: bal?.scannedTo ?: "—"} · ${info?.outputs ?: "—"} unspent",
            onRescan,
        )
        Text("Trusted node", color = Dim, fontSize = 12.sp, modifier = Modifier.padding(top = 16.dp))
        OutlinedTextField(node, onNode, modifier = Modifier.fillMaxWidth())
        Text("This phone believes this node. Tip ${bal?.tipHeight ?: "—"}. $status", color = Dim, fontSize = 12.sp)
        Text(privacyWarning(), color = Dim, fontSize = 12.sp, modifier = Modifier.padding(top = 16.dp, bottom = 16.dp))
        if (!pendingWipe) {
            OutlinedButton(onClick = { pendingWipe = true }, modifier = Modifier.fillMaxWidth()) { Text("Remove wallet from this phone") }
        } else {
            Text("This deletes the seed file on this phone. You need the 24 words to get it back.", color = Bad, fontSize = 13.sp)
            Button(onClick = onWipe, colors = ButtonDefaults.buttonColors(containerColor = Bad), modifier = Modifier.fillMaxWidth()) {
                Text("Delete seed from this phone")
            }
        }
        Text("0.7.0 · protocol v8 · not 100% anonymous", color = Dim, fontSize = 11.sp, modifier = Modifier.padding(top = 20.dp))
    }
}

@Composable
private fun ReceivePane(address: String, onCopy: (String) -> Unit, onBack: () -> Unit) {
    val bmp = remember(address) { qrBitmap(address) }
    Column(Modifier.padding(20.dp), horizontalAlignment = Alignment.CenterHorizontally) {
        TextButton(onClick = onBack, modifier = Modifier.align(Alignment.Start)) { Text("← Wallet", color = Dim) }
        Text("Receive", color = TextCol, fontSize = 28.sp, fontWeight = FontWeight.Bold, modifier = Modifier.align(Alignment.Start))
        Text("Share this address. Receiving works while this device is off.", color = Dim, fontSize = 13.sp, modifier = Modifier.align(Alignment.Start).padding(bottom = 16.dp))
        bmp?.let {
            Surface(shape = RoundedCornerShape(18.dp), color = Color(0xFFF4F0FF)) {
                Image(it.asImageBitmap(), "QR", modifier = Modifier.size(240.dp).padding(14.dp))
            }
        }
        Spacer(Modifier.height(16.dp))
        Surface(shape = RoundedCornerShape(16.dp), color = Card, modifier = Modifier.fillMaxWidth()) {
            SelectionContainer { Text(address, color = TextCol, fontFamily = FontFamily.Monospace, fontSize = 13.sp, modifier = Modifier.padding(14.dp)) }
        }
        Button(onClick = { onCopy(address) }, modifier = Modifier.fillMaxWidth().padding(top = 12.dp), colors = ButtonDefaults.buttonColors(containerColor = Accent)) {
            Icon(Icons.Outlined.ContentCopy, null, modifier = Modifier.size(16.dp))
            Spacer(Modifier.width(8.dp))
            Text("Copy address")
        }
    }
}

@Composable
private fun SendPane(
    to: String,
    amt: String,
    memo: String,
    fee: String,
    available: String?,
    onTo: (String) -> Unit,
    onAmt: (String) -> Unit,
    onMemo: (String) -> Unit,
    onBack: () -> Unit,
    onSend: () -> Unit,
) {
    var confirm by remember { mutableStateOf(false) }
    Column(Modifier.padding(20.dp)) {
        TextButton(onClick = onBack) { Text("← Wallet", color = Dim) }
        Text("Send", color = TextCol, fontSize = 28.sp, fontWeight = FontWeight.Bold)
        Text(
            "Wrong address = gone forever. Fee ${pretty(fee)} NIGHT, burned while blocks still pay a subsidy. Spendable ${pretty(available)} NIGHT.",
            color = Dim,
            fontSize = 13.sp,
            modifier = Modifier.padding(bottom = 12.dp),
        )
        OutlinedTextField(to, onTo, label = { Text("nf1…") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(amt, onAmt, label = { Text("Amount") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(memo, onMemo, label = { Text("Memo (optional)") }, modifier = Modifier.fillMaxWidth())
        if (!confirm) {
            Button(
                onClick = { confirm = true },
                enabled = to.startsWith("nf1") && amt.isNotBlank(),
                modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
                colors = ButtonDefaults.buttonColors(containerColor = Accent),
            ) { Text("Review") }
        } else {
            Text("Send $amt NIGHT plus ${pretty(fee)} fee to ${to.take(10)}…${to.takeLast(6)}?", color = TextCol, modifier = Modifier.padding(top = 12.dp))
            Button(onClick = onSend, modifier = Modifier.fillMaxWidth(), colors = ButtonDefaults.buttonColors(containerColor = Accent)) { Text("Confirm send") }
        }
    }
}

@Composable
private fun SecretPane(
    title: String,
    body: String,
    secret: String,
    onCopy: (String) -> Unit,
    onBack: () -> Unit,
    revealFirst: Boolean = false,
) {
    var show by remember { mutableStateOf(revealFirst) }
    Column(Modifier.padding(20.dp)) {
        TextButton(onClick = onBack) { Text("← Settings", color = Dim) }
        Text(title, color = TextCol, fontSize = 28.sp, fontWeight = FontWeight.Bold)
        Text(body, color = Dim, fontSize = 13.sp, modifier = Modifier.padding(bottom = 12.dp))
        var copied by remember { mutableStateOf(false) }
        if (!show) {
            Button(onClick = { show = true }, modifier = Modifier.fillMaxWidth(), colors = ButtonDefaults.buttonColors(containerColor = Accent)) {
                Text("Show")
            }
        } else {
            val words = secret.trim().split(Regex("\\s+"))
            if (words.size >= 12) WordGrid(words) else {
                Surface(shape = RoundedCornerShape(16.dp), color = Card, modifier = Modifier.fillMaxWidth()) {
                    SelectionContainer {
                        Text(secret, color = TextCol, fontFamily = FontFamily.Monospace, fontSize = 15.sp, modifier = Modifier.padding(14.dp))
                    }
                }
            }
            Button(
                onClick = { onCopy(secret); copied = true },
                modifier = Modifier.fillMaxWidth().padding(top = 12.dp),
            ) { Text("Copy all 24 words") }
            if (copied) Text("Copied. Clear the clipboard when you have stored them.", color = Ok, fontSize = 13.sp, modifier = Modifier.padding(top = 6.dp))
        }
    }
}

@Composable
private fun TxRow(e: HistoryView) {
    val incoming = e.direction == "Received" || e.direction == "Mined"
    Row(Modifier.fillMaxWidth().padding(vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
        Surface(shape = CircleShape, color = Color(0xFF1D1830), modifier = Modifier.size(38.dp)) {
            Box(contentAlignment = Alignment.Center) { Text(if (incoming) "↓" else "↑") }
        }
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            Text(e.direction, color = TextCol, fontWeight = FontWeight.SemiBold)
            val extra = if (e.pending) "pending" else e.height?.let { "#$it" } ?: ""
            Text(listOfNotNull(extra.ifBlank { null }, e.memo.ifBlank { null }).joinToString(" · "), color = Dim, fontSize = 12.sp)
        }
        Text(
            (if (incoming) "+" else "−") + pretty(e.amount) + " NIGHT",
            color = if (incoming) Ok else TextCol,
            fontWeight = FontWeight.SemiBold,
            fontSize = 13.sp,
        )
    }
}

@Composable
private fun Kv(k: String, v: String) {
    Row(Modifier.fillMaxWidth().padding(vertical = 3.dp), horizontalArrangement = Arrangement.SpaceBetween) {
        Text(k, color = Dim, fontSize = 13.sp)
        Text(v, color = TextCol, fontSize = 13.sp)
    }
}

@Composable
private fun SettingBtn(title: String, sub: String, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(14.dp),
        color = Card,
        modifier = Modifier.fillMaxWidth().padding(vertical = 5.dp),
    ) {
        Column(Modifier.padding(14.dp)) {
            Text(title, color = TextCol, fontWeight = FontWeight.SemiBold)
            Text(sub, color = Dim, fontSize = 12.sp)
        }
    }
}

private fun pretty(s: String?): String {
    if (s.isNullOrBlank()) return "—"
    val parts = s.split('.', limit = 2)
    val whole = parts[0].reversed().chunked(3).joinToString(",").reversed()
    val frac = parts.getOrNull(1)?.trimEnd('0').orEmpty()
    return if (frac.isEmpty()) "$whole.00" else "$whole.$frac"
}

private fun qrBitmap(text: String): Bitmap? {
    if (text.isBlank()) return null
    return runCatching {
        val bits = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 512, 512)
        val w = bits.width
        val h = bits.height
        val px = IntArray(w * h) { i ->
            if (bits[i % w, i / w]) 0xFF12091C.toInt() else 0xFFF4F0FF.toInt()
        }
        Bitmap.createBitmap(px, w, h, Bitmap.Config.ARGB_8888)
    }.getOrNull()
}
