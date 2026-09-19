# Core UI — lokale Dev 1.0.0-dev.2

Prüfung: 12.–13. September 2026, Apple Silicon/macOS. Kein Produktionsrelease,
kein unabhängiger Audit und keine Zusage vollständiger Sicherheit.

## Ergebnis

Die ursprüngliche Nightfall-Palette bleibt exakt erhalten: violette Flächen,
Pink-/Cyan-Verlauf, bestehende Text- und Statusfarben. Ein Regressionstest
sichert die Markenfarben ab; Kontrasttests prüfen Text und Verlaufsschrift.
Der zwischenzeitliche Graphit-Entwurf wurde verworfen.

Überarbeitet sind Navigation, Kopfzeilen, Typografie, Karten, Eingaben,
Fokusdarstellung und Copy-Bedienung. macOS-Schriften werden lokal aus dem
Betriebssystem gelesen, nicht in der App weiterverteilt.

Die neuen Arbeitsbereiche sind:

- Send: Payment / Offline signing · Air.
- Receive: Receive NIGHT / Invoices · Counter.
- Activity: Transactions / Proof Cards.
- Settings: Vault / Recovery / Contacts / Node / About.

Backup- und Resync-Verknüpfungen öffnen gezielt den richtigen Settings-Bereich.
Das Öffnen des Backup-Bereichs zeigt keine Recovery-Wörter automatisch an.
Counter verlangt vor dem Entfernen eines Datensatzes eine Bestätigung.
Pending-Zahlungen werden nicht aufgrund ihres Alters als gescheitert erklärt;
die UI empfiehlt weder Doppelzahlungen noch das Freigeben ihrer Reservierungen.
Scanfehler bleiben sichtbar, ohne bei jedem Scan neue identische Toasts zu stapeln.

Swap zeigt Voraussetzungen und Sicherheitsgates, nur die rollenspezifisch
relevanten Eingaben, Netzwerk-/Integritätsprüfung der Pakete und verständliche
nächste Schritte. Paketintegrität wird nicht als abgeschlossener Handschlag
ausgegeben. Kompensation mit permanentem NIGHT-Lock heißt nicht „erfolgreich“
oder „NIGHT zurückerstattet“. Mainnet und Vault-Ausführung bleiben gesperrt.

## Übergebenes Paket

Der endgültige Build liegt unter:

`dev-builds/macos-dev-1.0.0-arm64-nSJhXO/`

Enthalten: `NIGHTFALL Dev 1.0.0.app`,
`NIGHTFALL-Dev-1.0.0-dev.2-macOS-arm64.zip`, `SHA256SUMS.txt` und
`READ-ME-FIRST.md`. Frühere Zwischenstände in anderen Ausgabeordnern sind
nicht das hier geprüfte Übergabepaket.

- Bundle-ID: `cash.nightfall.wallet.dev100preview`.
- Bundle-Version: `1.0.0`; Entwicklungsanzeige: `1.0.0-dev.2`.
- Profil: `isolated-1.0.0-dev.2`, Devnet, ARM64.
- Eigener Datenunterordner: `wallet-1.0.0-dev.2`.
- Lokale Ad-hoc-Signatur; nicht notarisiert.
- Mainnet/Testnet werden vor dem Datenzugriff verweigert. Der Finder-Launcher
  lässt keine Netzwerk-/Datenpfad-Overrides zu.
- Mainnet-Umgebungsvariablen aktivieren dieses Profil nicht. Es übernimmt
  keine bestehenden Wallets, öffentlichen Seeds, Connect- oder Proxy-Variablen.

Die Build-Binary hat SHA-256
`7018d5a6935afbe5f23e0a7cdbeaf9e6648a6fbcb759d038c7fa4da19f747aae`.
Die ZIP-Prüfsumme steht in der beiliegenden `SHA256SUMS.txt`.

## Nachweise

| Prüfung | Ergebnis / Grenze |
| --- | --- |
| Core, vollständige normale Suite | 68 bestanden, 2 als ignoriert markiert; der Vault-Node-Unterprozess wird vom bestandenen Lifecycle-Test zweimal aufgerufen. Core-Live-Swap nicht ausgeführt. |
| Wallet, normale Suite | 130 bestanden, 1 Unterprozess-Helfer regulär ignoriert und vom Elterntest aufgerufen. |
| Swap, normale Unit-/Integrationstests | 148 bestanden; explizite Live-/Langzeit-/bitcoind-Prüfungen nicht durch diesen Lauf abgedeckt. |
| Core Clippy, alle Targets, Warnungen als Fehler | Bestanden. |
| Core rustfmt / Diff-Whitespace | Bestanden. |
| Paketprofil und veröffentlichter Kanal | Bestanden; veröffentlichte 0.9.5-Dateien unverändert. |
| ARM64, plist, codesign und ZIP-SHA-256 | Bestanden. |
| Start mit Mainnet/Testnet bzw. Launcher-Override | Abgewiesen; angegebene Test-Datenordner wurden nicht angelegt. |
| Layout | Alle acht Seiten; alle neuen Unterbereiche; 620/884/1180 Punkte Inhalt; egui-Fallback- und native macOS-Schriften. |

Die Wallet-/Swap-Suiten wurden vor den abschließenden reinen UI-Korrekturen
ausgeführt; ihre Backend-Dateien wurden durch diese UI-Arbeit nicht geändert.
Nach den letzten UI-Korrekturen wurden Core-Suite und Clippy erneut bestanden.
Ein kurzfristiger Kompilierfehler im neu ergänzten Test wurde korrigiert; er
betraf ausschließlich dessen Assertion, nicht die ausgelieferte Binary.

Native Bildschirmprüfung: alle acht Seiten sowie Air, Counter, Proof, Recovery,
Contacts, Node und About einschließlich ihrer gescrollten Abschnitte. Die
Ansichten liegen unter `output/ui-dev2-violet-review/`; finale Aufnahmen nach
den letzten Navigation-/Devnet-Korrekturen unter `output/ui-dev2-final/`.
Aufnahmen verwenden ausschließlich eine öffentliche Nullseed-Testwallet auf
einer leeren lokalen Devnet-Chain. Die Scanwarnung bei Block 0 ist sichtbar;
es wurde für die Bilder kein erfolgreicher Scan vorgetäuscht.

Zusätzlich im finalen Bundle bedient: Receive → Counter, lokale Rechnung über
1,50 Test-NIGHT mit eigener Referenz erstellt, Entfernen-Dialog geöffnet und
mit „Keep invoice“ abgebrochen; Activity → Proof Cards, ungültiges JSON als
ungültig abgewiesen, Clear entfernt Dokument und Befund; Settings → Recovery
ohne Geheimnisanzeige; lokale Network-Anzeige ohne falsches Tor-Versprechen.
Die native App-Steuerung machte die Kontrolle der tatsächlich bedienbaren
Controls möglich; Eingabefelder wurden bei unzuverlässigem AX-Fokus anhand
des aktuellen Screenshots fokussiert. Das eigene Testfenster wurde geschlossen.

## Offen und ausdrücklich nicht bewiesen

Kein neuer manueller Komplettlauf sämtlicher Onboarding-, Passwortwechsel-,
Backup-/Restore- und Zahlungsszenarien in dieser UI-Runde. Die automatisierten
Core-Tests prüfen Teile dieser Abläufe einschließlich lokalem Node, Zahlung,
Bestätigung und Neustart; sie ersetzen keine Geräte-/Gegenparteitests.

Vault-Swap-Geheimnisse und Fristen, integrierte Gebührenleiter, permanentes
NIGHT-Lock-Risiko, öffentliche Langzeittests und externe Kryptografieprüfung
bleiben offen. Kamera/mobile Air-Gegenstelle, unabhängige View-only-Kasse und
vollständige Browser-/Webwallet-Abnahme werden durch diese Desktop-Vorschau
nicht fertiggestellt. Die Release-Checkliste bleibt maßgeblich.

Keine Mainnet-Wallet geöffnet, gestoppt, migriert oder ersetzt. Keine echten
Seeds/Backups gelesen. Keine GitHub-Pushes, Websiteänderungen oder Deployments.
Testdateien enthalten nur öffentliche Fixtures und wurden nicht als Nutzerdaten
gelöscht. Anleitung: `docs/DEV-WALLET-1.0.0-dev.2.md`.
