# Core UI review — 0.9.4-dev.1

Reference: the repository's `website/public/wallet/style.css` and wallet structure.
The webwallet itself and production website were not changed for this review.

| Page | Reviewed / improved |
|---|---|
| Dashboard | Balance hierarchy, responsive metrics, secondary-text contrast |
| Send | Full-address review, disabled background form, inactive buttons, Escape |
| Receive | QR/address grouping, column spacing, receiving status, accurate privacy copy |
| Activity | Responsive totals with NIGHT units, useful filtered empty state, reset |
| Mining | Responsive performance metrics, CPU controls, readable explanatory text |
| Network | Explicit disconnected state rather than an unsupported “mining solo” claim |
| Swap | Active trades first, background health checks, confirmation/recovery feedback |
| Settings | Hide keys on exit, connection-check navigation, active-swap maintenance guards |

Shared changes: webwallet surface palette; page introductions; separate scroll
state per page; accessible labels and visible keyboard focus; no doubled column gap.

Automated coverage: all eight pages render without horizontal overflow at
620/884/1180-pixel content widths (empty/disconnected fixtures), primary buttons
cannot dispatch clicks when disabled directly or by a parent, secondary text
tokens meet 4.5:1 on base surfaces. These checks do not exhaust populated histories,
all operating systems, screen readers or every font/display scaling combination.

Native capture initially failed with ScreenCaptureKit -3811, then recovered.
All eight pages of the installed macOS Dev app were visually reviewed in the
empty/disconnected devnet state. This is not Windows/Linux visual acceptance.
