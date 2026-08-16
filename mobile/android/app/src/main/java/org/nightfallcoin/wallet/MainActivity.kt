package org.nightfallcoin.wallet

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.nightfall_mobile.BalanceView
import uniffi.nightfall_mobile.HistoryView
import uniffi.nightfall_mobile.MobileWallet
import uniffi.nightfall_mobile.defaultNode
import uniffi.nightfall_mobile.privacyWarning
import uniffi.nightfall_mobile.walletExists

private val Bg = Color(0xFF0E0B18)
private val Card = Color(0xFF1A1626)
private val Accent = Color(0xFFB845D8)
private val Text = Color(0xFFF4F0FF)
private val Dim = Color(0xFF9A92B3)

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val dir = filesDir.absolutePath
        setContent {
            MaterialTheme(colorScheme = darkColorScheme(background = Bg, surface = Card, primary = Accent)) {
                Surface(Modifier.fillMaxSize().background(Bg), color = Bg) {
                    App(dir)
                }
            }
        }
    }
}

@Composable
private fun App(dir: String) {
    var wallet by remember { mutableStateOf<MobileWallet?>(null) }
    var phrase by remember { mutableStateOf<String?>(null) }
    var confirmed by remember { mutableStateOf(false) }
    var err by remember { mutableStateOf<String?>(null) }

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
                runCatching {
                    val tip = 0uL
                    val w = MobileWallet.create(dir, "mainnet", tip)
                    phrase = w.recoveryPhrase()
                    wallet = w
                }.onFailure { err = it.message }
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
        else -> Home(wallet!!)
    }
}

@Composable
private fun Onboard(err: String?, onCreate: () -> Unit, onRestore: (String, Long) -> Unit) {
    var restore by remember { mutableStateOf(false) }
    var words by remember { mutableStateOf("") }
    var height by remember { mutableStateOf("0") }
    Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState())) {
        Text("NIGHT", color = Accent, fontSize = 28.sp, fontWeight = FontWeight.Bold)
        Text("Money that refuses to snitch.", color = Dim, modifier = Modifier.padding(top = 6.dp))
        Spacer(Modifier.height(24.dp))
        Text(privacyWarning(), color = Dim, fontSize = 13.sp)
        Spacer(Modifier.height(28.dp))
        Button(onClick = onCreate, colors = ButtonDefaults.buttonColors(containerColor = Accent)) {
            Text("Create a wallet")
        }
        TextButton(onClick = { restore = !restore }) { Text("I have 24 words", color = Dim) }
        if (restore) {
            OutlinedTextField(words, { words = it }, label = { Text("Recovery phrase") }, modifier = Modifier.fillMaxWidth())
            OutlinedTextField(height, { height = it }, label = { Text("Birth height (0 if unsure)") }, modifier = Modifier.fillMaxWidth())
            Text("A number that is too high silently misses coins. Too low only costs time.", color = Dim, fontSize = 12.sp)
            Button(onClick = { onRestore(words, height.toLongOrNull() ?: 0L) }) { Text("Restore") }
        }
        err?.let { Text(it, color = Color(0xFFFF6B8A), modifier = Modifier.padding(top = 16.dp)) }
    }
}

@Composable
private fun Backup(phrase: String, err: String?, onDone: () -> Unit) {
    val words = phrase.trim().split(Regex("\\s+"))
    var a by remember { mutableStateOf("") }
    var b by remember { mutableStateOf("") }
    val i = 3
    val j = 17
    Column(Modifier.fillMaxSize().padding(24.dp).verticalScroll(rememberScrollState())) {
        Text("Write these 24 words down.", color = Text, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
        Text("On paper. Not a photo, not the cloud. Nobody can reset this.", color = Dim, modifier = Modifier.padding(vertical = 8.dp))
        SelectionContainer {
            Text(phrase, color = Text, fontFamily = FontFamily.Monospace, fontSize = 16.sp, modifier = Modifier.padding(vertical = 12.dp))
        }
        Text("Type word ${i + 1} and word ${j + 1}.", color = Dim)
        OutlinedTextField(a, { a = it }, label = { Text("Word ${i + 1}") })
        OutlinedTextField(b, { b = it }, label = { Text("Word ${j + 1}") })
        val ok = words.getOrNull(i).equals(a.trim(), true) && words.getOrNull(j).equals(b.trim(), true)
        Button(onClick = onDone, enabled = ok, modifier = Modifier.padding(top = 16.dp)) { Text("I have the words") }
        err?.let { Text(it, color = Color(0xFFFF6B8A)) }
    }
}

@Composable
private fun Home(wallet: MobileWallet) {
    val scope = rememberCoroutineScope()
    var node by remember { mutableStateOf(defaultNode()) }
    var address by remember { mutableStateOf("") }
    var bal by remember { mutableStateOf<BalanceView?>(null) }
    var hist by remember { mutableStateOf(listOf<HistoryView>()) }
    var status by remember { mutableStateOf("tap Sync") }
    var to by remember { mutableStateOf("") }
    var amt by remember { mutableStateOf("") }
    var memo by remember { mutableStateOf("") }
    val clip = LocalClipboardManager.current

    fun refresh() {
        scope.launch {
            status = "syncing…"
            val r = withContext(Dispatchers.IO) {
                runCatching {
                    val found = wallet.sync(node)
                    address = wallet.address()
                    bal = wallet.balance(node)
                    hist = wallet.history()
                    found
                }
            }
            status = r.fold(
                { n -> if (n == 0u) "up to date" else "found $n new output(s)" },
                { e -> e.message ?: "sync failed" },
            )
        }
    }

    LaunchedEffect(Unit) {
        address = runCatching { wallet.address() }.getOrDefault("")
        refresh()
    }

    Column(Modifier.fillMaxSize().padding(20.dp).verticalScroll(rememberScrollState())) {
        Text("NIGHT", color = Accent, fontWeight = FontWeight.Bold, fontSize = 18.sp)
        Text(bal?.total ?: "—", color = Text, fontSize = 32.sp, fontFamily = FontFamily.Monospace)
        Text(
            "spendable ${bal?.available ?: "—"}  ·  unlocking ${bal?.immature ?: "—"}",
            color = Dim,
            fontSize = 13.sp,
        )
        Text(status, color = Dim, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp, bottom = 12.dp))
        Row {
            Button(onClick = { refresh() }) { Text("Sync") }
            Spacer(Modifier.width(8.dp))
            OutlinedButton(onClick = { clip.setText(AnnotatedString(address)) }) { Text("Copy address") }
        }
        Spacer(Modifier.height(12.dp))
        Text("Your address", color = Dim, fontSize = 12.sp)
        SelectionContainer { Text(address, color = Text, fontFamily = FontFamily.Monospace, fontSize = 12.sp) }
        Spacer(Modifier.height(20.dp))
        Text("Send", color = Text, fontWeight = FontWeight.SemiBold)
        Text("Wrong address = gone forever. Fee 0.001 NIGHT, burned while blocks still pay a subsidy.", color = Dim, fontSize = 12.sp)
        OutlinedTextField(to, { to = it }, label = { Text("nf1…") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(amt, { amt = it }, label = { Text("Amount") }, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(memo, { memo = it }, label = { Text("Memo (optional)") }, modifier = Modifier.fillMaxWidth())
        Button(
            onClick = {
                scope.launch {
                    status = "sending…"
                    val r = withContext(Dispatchers.IO) {
                        runCatching { wallet.send(node, to.trim(), amt.trim(), memo) }
                    }
                    status = r.fold({ "sent $it" }, { it.message ?: "send failed" })
                    if (r.isSuccess) refresh()
                }
            },
            modifier = Modifier.padding(top = 8.dp),
        ) { Text("Send") }
        Spacer(Modifier.height(20.dp))
        Text("Activity", color = Text, fontWeight = FontWeight.SemiBold)
        hist.forEach { e ->
            Text(
                "${e.direction}  ${e.amount}" + if (e.pending) "  (pending)" else e.height?.let { "  #$it" }.orEmpty(),
                color = Text,
                fontFamily = FontFamily.Monospace,
                fontSize = 13.sp,
            )
            if (e.memo.isNotBlank()) Text(e.memo, color = Dim, fontSize = 12.sp)
        }
        Spacer(Modifier.height(20.dp))
        Text("Node this phone trusts", color = Dim, fontSize = 12.sp)
        OutlinedTextField(node, { node = it }, modifier = Modifier.fillMaxWidth())
        Text(privacyWarning(), color = Dim, fontSize = 11.sp, modifier = Modifier.padding(top = 12.dp, bottom = 24.dp))
    }
}
