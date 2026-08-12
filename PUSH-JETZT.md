# Letzter Schritt: anmelden und pushen

Alles ist vorbereitet. Das Repo in diesem Ordner ist initialisiert, committet
und zeigt bereits auf `instinctes/nightfall`. Es fehlt nur deine Anmeldung —
die kann ich nicht für dich machen, und das ist auch gut so.

## 1. Bei GitHub anmelden

```bash
gh auth login
```

Wähle: **GitHub.com** → **HTTPS** → **Login with a web browser**.
Der Code wird angezeigt, Browser öffnet sich, fertig.

## 2. Repo anlegen und pushen — ein Befehl

```bash
cd ~/Documents/0_Crypto/github
gh repo create instinctes/nightfall --public --source=. --push \
  --description "Money that refuses to snitch. A privacy Layer-1 with a supply anyone can prove."
```

Das legt das öffentliche Repo an, verbindet es und pusht `main`.

Falls du das Repo lieber von Hand auf github.com anlegst (leer, **ohne**
README/Lizenz/gitignore — die sind hier schon drin), genügt danach:

```bash
git push -u origin main
```

## 3. Direkt danach

```bash
gh repo edit instinctes/nightfall \
  --homepage "https://nightfallcoin.husref-huremovic.workers.dev" \
  --add-topic cryptocurrency --add-topic privacy --add-topic blockchain \
  --add-topic rust --add-topic mimblewimble --add-topic bulletproofs \
  --add-topic proof-of-work --add-topic argon2 \
  --enable-issues --enable-wiki=false
```

Dann im Browser unter **Settings → Code security**: *Private vulnerability
reporting* einschalten, damit `SECURITY.md` nicht der einzige Meldeweg ist.

Und unter **Settings → Branches**: Schutzregel für `main`, die den CI-Check
verlangt. Die Exploit-Suite ist der Grund, warum es CI gibt — an einem roten
Build sollte nichts vorbeikommen.

## 4. Erstes Release

```bash
cd ~/Documents/0_Crypto
shasum -a 256 wallets/*.dmg wallets/windows-x64/*.exe

gh release create v0.3.0 \
  wallets/NIGHTFALLCOIN-Core-macOS-arm64.dmg \
  wallets/NIGHTFALLCOIN-Core-macOS-intel.dmg \
  wallets/windows-x64/nightfall-core.exe \
  wallets/windows-x64/nightfalld.exe \
  wallets/windows-x64/nightfall-wallet.exe \
  --title "v0.3.0 — Nightproof-beta" \
  --notes "Erste öffentliche Version. Prüfsummen siehe unten."
```

**Die Prüfsummen gehören in die Release-Notes.** Die Binaries sind unsigniert;
eine SHA-256 ist die einzige Möglichkeit, deinen Build von einem
untergeschobenen zu unterscheiden.

Schreib in die Notes auch klar hin, dass die Builds unsigniert sind und wie man
an Gatekeeper und SmartScreen vorbeikommt. Wer eine Sicherheitswarnung ohne
Erklärung wegklicken soll, lernt Sicherheitswarnungen wegzuklicken.

## Was im Repo landet

105 Dateien, 3,8 MB. Geprüft und bestätigt ohne:
Seeds, Kettendaten, Binaries, private Schlüssel, API-Tokens.

Deine Wallet-Seeds liegen in `~/Library/Application Support/nightfall/` und
wurden nie hierher kopiert.
