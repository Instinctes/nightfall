# NIGHTFALL 1.0.0-dev.1 — lokale Apple-Silicon-Testversion

Unvollständige Entwicklungsvorschau vom 10. September 2026, kein Mainnet-Release.
Der veröffentlichte Wallet-Kanal bleibt 0.9.5. Nur die Core-GUI in diesem Bundle
trägt die Entwicklungsnummer; Website, Downloads und Workspace-Version bleiben
unverändert.

## Starten

ZIP entpacken und **NIGHTFALL 1.0 Dev.app** öffnen. Sie kann im entpackten Ordner
bleiben; die bestehende Core-App nicht ersetzen. Nur für Apple Silicon (ARM64).
Lokal ad-hoc signiert, nicht von Apple notarisiert. Bei einer macOS-Warnung
nicht die systemweiten Sicherheitsfunktionen deaktivieren.

Die App nutzt Devnet und einen eigenen `wallet-1.0-dev`-Unterordner des
Devnet-Datenverzeichnisses. Mainnet/Testnet werden bereits vor dem Öffnen eines
Datenordners abgewiesen. Node-Ports werden automatisch auf Loopback vergeben;
keine Seed-/Peer-Verzeichnis-Abfrage und keine übernommenen Connect- oder
Proxy-Umgebungsvariablen. Es ist ein lokales Testnetz, keine Verbindung zur
öffentlichen Mainnet-Chain. Bestehende Mainnet-Apps und Daten bleiben unberührt.

**Nur eine neue Testwallet verwenden. Niemals echte Recovery-Wörter, Seeds oder
Backups mit Guthaben in eine Entwicklungsversion eingeben. Devnet-Coins sind
keine echten NIGHT.**

## Was du testen kannst

1. Neue Testwallet erstellen, die 24 Wörter sichern und vollständig überprüfen;
   Passwort mit mindestens 12 Zeichen setzen. Danach ausdrücklich entsperren.
2. Settings: Vault sperren/entsperren, Passwort ändern, automatische Sperre bei
   Fokusverlust und nach fünf Minuten Inaktivität prüfen.
3. Mining starten, um lokale Testcoins zu erzeugen. Coinbase-Ausgänge werden
   nach 10 Devnet-Blöcken reif. Mining anschließend wieder stoppen.
4. Receive/Send/Activity sowie Recovery Studio ausprobieren. Zahlungen an die
   eigene Adresse werden abgewiesen. Ein echter lokaler Zahlungstest benötigt
   eine zweite Testadresse auf derselben lokalen Chain. Der automatisierte
   Node-Test prüft genau das mit zwei getrennten Wallets. Die GUI-Vorschau
   verbindet nicht automatisch zwei getrennt gestartete lokale Devnet-Nodes.
5. In Settings ein verschlüsseltes Backup in einen neuen absoluten Dateipfad
   außerhalb des Wallet-Datenordners exportieren und lesend überprüfen.
   Vorhandene Backups werden nicht überschrieben. Alte Backups behalten ihr
   bisheriges Passwort; unverändert aufbewahren.
6. Neu starten: Die Wallet muss gesperrt öffnen. Zum separaten Restore-Test
   kann das Binary mit `--network devnet --datadir /absoluter/neuer/testordner`
   gestartet werden. Nie einen bestehenden Wallet-Ordner angeben.
   Backup-Recovery stellt ausschließlich Schlüssel wieder her; alte lokale
   Historie, Reservierungen und gespeicherte Pending-TX werden nicht importiert.

## Noch nicht fertig

Full-State-Backup-Import, vollständige Reorg-/Langscan-Härtung, Browser-Vault,
Windows-Persistenz und die 1.0.0-Releaseprüfung sind offen. **Atomic Swaps
wurden am 18. September 2026 aus dem Projekt entfernt** — es gibt keine
Swap-Seite, keine bitcoind-Konfiguration und keinen Swap-Code mehr; Begründung
in `docs/SWAP-WITHDRAWN.md`. Keine Konsensregel hat sich dadurch geändert.
Diese Vorschau ist keine Sicherheitsgarantie und kein unabhängiger Audit.

Während eines Wallet-Scans zeigt die GUI eine Fortschrittsansicht ohne
Zwischenstand-Guthaben. Sperren blendet Walletinformationen sofort aus; ein
bereits laufender Scan muss enden, bevor seine Schlüssel im RAM freigegeben
werden. Die vollständige Seitennavigation während langer Scans ist noch kein
abgeschlossenes Feature.

Manueller Rescan/Resync ist bei Pending-Zahlungen und reservierten Outputs
gesperrt, damit deren lokale Sicherungsdaten nicht verschwinden. Eine
Wallet-Datei aus einem alten experimentellen Swap-Build blockiert Rescan und
Restore weiterhin; das ist Absicht.
Ein Rescan löscht lokale Historie und rekonstruiert Chain-Daten, nicht sämtliche
Belege/Notizen. Vorher ein verschlüsseltes Backup exportieren und behalten.
