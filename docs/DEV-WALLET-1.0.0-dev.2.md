# NIGHTFALL Dev 1.0.0 — lokale Apple-Silicon-Vorschau

Entwicklungsvorschau, Stand 12. September 2026. **Kein Mainnet-Release und keine
Sicherheitsgarantie.** Die interne Versionsanzeige lautet `1.0.0-dev.2`; die
macOS-Bundle-Version lautet `1.0.0`. Der veröffentlichte Kanal bleibt 0.9.5.
Diese Vorschau ist noch nicht die endgültige Wallet 1.0.0.

## Sicher starten

ZIP entpacken und **NIGHTFALL Dev 1.0.0.app** öffnen. Die App darf im entpackten
Ordner bleiben. Weder die vorhandene Core-App noch eine ältere Dev-App ersetzen.
Dieses ARM64-Paket benötigt einen Apple-Silicon-Mac. Es ist lokal ad-hoc signiert,
nicht von Apple notarisiert. macOS-Sicherheitsfunktionen nicht global abschalten.

Nur eine **neue Testwallet** erstellen. Niemals echte Recovery-Wörter, Seeds,
View Keys oder Backups mit Guthaben eingeben. Devnet-Coins sind keine echten
NIGHT; sie haben hier ausschließlich Testzwecke.

Die App startet auf Devnet in einem eigenen `wallet-1.0.0-dev.2`-Unterordner des
Devnet-Datenverzeichnisses. Unter macOS ist das derzeit:

`~/Library/Application Support/nightfall/devnet/n8/wallet-1.0.0-dev.2`

Mainnet, die bisherige `wallet-1.0-dev`-Vorschau und installierte Apps werden
nicht übernommen. Die Bundle-ID ist `cash.nightfall.wallet.dev100preview`. Auch eine
beim Bauen geerbte Variable `NIGHTFALL_DEV_MAINNET` schaltet Mainnet für dieses
Profil nicht frei. Die App verweigert Mainnet/Testnet vor dem Öffnen eines
Datenordners. Ihr Finder-Launcher lässt keine Netzwerk-/Datenpfad-Overrides zu.

Devnet ist lokal isoliert: automatische Loopback-Ports, keine öffentlichen
Seed-/Peer-Verzeichnisse und keine übernommenen Connect-/Proxy-Variablen.
Zwei parallel gestartete lokale Devnet-Nodes bilden deshalb nicht automatisch
eine gemeinsame Chain. Während des Tests bleibt die veröffentlichte Mainnet-
Wallet samt Daten unverändert.

## Testablauf

1. **Onboarding und Vault:** neue Testwallet erstellen; 24 Wörter vollständig
   bestätigen; Passwort mit mindestens 12 Zeichen setzen; bewusst entsperren.
   Falsches Passwort, Sperren, Fokusverlust, Inaktivität und Neustart prüfen.
2. **Alle acht Seiten:** Dashboard, Send, Receive, Activity, Mining, Network,
   Atomic Swaps und Settings ansehen. Fenster schmal/breit machen und jede
   Seite bis zum Ende scrollen. Texte, Beträge, Warnungen und Aktionen müssen
   lesbar bleiben; ein deaktivierter Ablauf darf keinen Erfolg behaupten.
3. **Lokale Coins:** Mining starten. Devnet-Coinbase wird nach zehn Blöcken
   reif. Mining danach stoppen. Ein Zahlungstest braucht eine andere Adresse
   auf derselben lokalen Chain; die eigene Adresse wird bewusst abgewiesen.
4. **Recovery:** ein verschlüsseltes Backup in einen neuen absoluten Dateipfad
   außerhalb des Datenordners exportieren und prüfen. Vorhandene Backups werden
   nicht überschrieben. Passwort wechseln: ein bereits exportiertes Backup
   behält sein damaliges Passwort; ein neues Backup separat exportieren.
5. **Import:** mit einem neuen, leeren Testordner Keys-only und Full-state
   getrennt prüfen. Keys-only rekonstruiert keine lokale Historie. Full-state
   übernimmt den gesicherten Zustand, aber importierte Pending-Zahlungen
   werden nicht automatisch erneut gesendet. Originalbackup aufbewahren.
6. **Pay und Proof Cards:** Requests erstellen/einlesen, Beträge und Netzwerk
   prüfen. Proof Cards exportieren/prüfen und eine manipulierte Kopie ablehnen
   lassen. Signatur, bewiesener Betrag und Chain-Bestätigung sind unterschiedliche
   Aussagen; die Karte muss ihre Grenzen anzeigen.
7. **Counter:** auf Receive Rechnungen mit eindeutiger Referenz erstellen.
   Gleiche Beträge dürfen nicht ohne passende Referenz als bezahlt gelten.
   Ablaufdatum, manuelles Schließen und Neustart prüfen. Counter in Core ist
   noch kein separat betreibbares View-only-Kassensystem.
8. **Air:** auf Send Intent-/Signed-Pakete und ihre Netzwerk-/Ablaufprüfungen
   prüfen. Die kalte Seite hält die Wallet, die Online-Seite den Node.
   Kamera und mobile Gegenstelle fehlen; der Transfer ist derzeit Text-/Datei-
   basiert. Ein realer Kalt-/Online-Test benötigt bewusst getrennte Testrollen.
9. **Atomic Swaps:** Status, Risiken, Vorbedingungen und Bedienführung prüfen.
   Die Überarbeitung hebt die Sicherheitsgates nicht auf: Mainnet bleibt
   gesperrt; Vault-Swap-Ausführung ist nicht freigegeben. Kein echter Bitcoin
   und keine echten NIGHT dürfen für diesen Test verwendet werden.

Für einen unabhängigen Restore-Test darf die innere Binary ausdrücklich mit
`--network devnet --datadir /absoluter/neuer/leerer/testordner` gestartet werden.
Der Ordner muss neu und ausschließlich für diese Vorschau bestimmt sein.
**Nie einen bestehenden Mainnet-, Stable-, älteren Dev- oder fremden Wallet-
Ordner angeben.** Der geschützte Finder-Launcher nimmt diese Overrides nicht an.

## Grenzen dieser Vorschau

Dies ist ein lokaler UX-/Funktionstest, kein fertiges 1.0.0-Release und kein
unabhängiger Audit. Automatisierte Tests ersetzen keine Prüfung aller Abläufe
auf echten Geräten und in adversarialen Netzwerksituationen.

Atomic Swaps sind weiterhin ein Forschungs-/Integrationsstrang. Insbesondere
Vault-Geheimnisse und Fristen, Fee-Ladder, das Risiko dauerhaft gesperrter NIGHT,
Langzeit-/öffentliche Gegenparteitests und unabhängige kryptografische Prüfung
sind keine durch dieses Paket gelösten Sicherheitszusagen. Die Oberfläche darf
erklären und vorbereiten, aber keine fehlende Freigabe vortäuschen.

Browser-Vault und Webwallet laufen nicht in diesem Desktop-Bundle. Deren noch
offene Browser-/Scan-/Send-Prüfungen werden nicht durch einen erfolgreichen
macOS-Test abgedeckt. Kamera-Import und mobile Air-Gegenstelle fehlen ebenfalls.

Ein manueller Rescan/Resync ist bei Pending-Zahlungen, reservierten Outputs oder
offenen Swaps gesperrt. Ein Rescan rekonstruiert Chain-Daten, nicht alle lokalen
Belege und Notizen. Vorher ein verschlüsseltes Backup exportieren und behalten.
Sperren blendet Walletinformationen aus; ein bereits laufender Scan muss enden,
bevor seine Schlüssel im RAM freigegeben werden.

## Paket prüfen und Rückmeldung geben

Neben der ZIP liegen `SHA256SUMS.txt` und diese Anleitung. Im Ausgabeordner:

```sh
shasum -a 256 -c SHA256SUMS.txt
```

Im Bundle dokumentiert `Contents/Resources/BUILD-INFO.txt` Version, Profil,
Netzwerk und Architektur. Die Prüfsumme erkennt Übertragungsfehler, ist aber
keine unabhängige Herausgeberbestätigung.

Bei Fehlern bitte Seite, genaue Schritte, Fenstergröße, sichtbare Fehlermeldung
und Screenshot notieren. Recovery-Wörter, Passwörter, View Keys und private
Backups niemals in Screenshots oder Fehlerberichte aufnehmen.
