//! One presentation per observed mining output. No wallet writes or timers on
//! background threads: egui's frame clock drives this small vector animation.
use eframe::egui::{self, Align2, Color32, RichText, Rounding, Stroke};
use nightfall_types::Amount;
use nightfall_wallet::{Direction, HistoryEntry};
use std::collections::{HashSet, VecDeque};

const DURATION: f64 = 2.8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reward {
    amount: u64,
}

#[derive(Default)]
pub struct Rewards {
    seen: HashSet<String>,
    primed: bool,
    pending: VecDeque<Reward>,
    active: Option<(Reward, f64)>,
}

impl Rewards {
    /// Seed history quietly after startup/unlock. Keep already seen IDs across
    /// rescans and lock/unlock so a temporarily orphaned output cannot chime twice.
    pub fn observe(&mut self, entries: &[HistoryEntry], quiet: bool) {
        let silent = quiet || !self.primed;
        for entry in entries
            .iter()
            .filter(|entry| entry.direction == Direction::Mined)
        {
            if self.seen.insert(entry.txid.clone()) && !silent && entry.amount > 0 {
                // Catch-up batches do not create an unbounded notification queue.
                if self.pending.len() < 32 {
                    self.pending.push_back(Reward {
                        amount: entry.amount,
                    });
                }
            }
        }
        self.primed = true;
    }

    pub fn conceal(&mut self) {
        self.pending.clear();
        self.active = None;
        self.primed = false;
    }

    /// Only the opt-in development screenshot harness calls this.
    pub fn preview(&mut self, now: f64) {
        self.active = Some((
            Reward {
                amount: 100_000_000,
            },
            now - 0.7,
        ));
    }

    fn advance(&mut self, now: f64) -> bool {
        if self
            .active
            .is_some_and(|(_, start)| now - start >= DURATION)
        {
            self.active = None;
        }
        if self.active.is_none() {
            if let Some(reward) = self.pending.pop_front() {
                self.active = Some((reward, now));
                return true;
            }
        }
        false
    }

    /// `top` is where the page content starts (below the top bar), so the chip
    /// sits inside the top-right corner of the first card, never over the header.
    pub fn show(&mut self, ctx: &egui::Context, sound_enabled: bool, top: f32) {
        let now = ctx.input(|input| input.time);
        if self.advance(now) && sound_enabled {
            crate::feedback::play_reward_chime();
        }
        let Some((reward, start)) = self.active else {
            return;
        };
        let elapsed = (now - start).max(0.0) as f32;
        let enter = (elapsed / 0.24).clamp(0.0, 1.0);
        let enter = 1.0 - (1.0 - enter).powi(3);
        let exit = ((DURATION as f32 - elapsed) / 0.45).clamp(0.0, 1.0);
        let opacity = enter * exit;
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        egui::Area::new(egui::Id::new("mining-reward"))
            .anchor(
                Align2::RIGHT_TOP,
                egui::vec2(-52.0, top + 30.0 - 8.0 * (1.0 - enter)),
            )
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(opacity);
                egui::Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(45, 34, 69, 238))
                    .stroke(Stroke::new(
                        1.0_f32,
                        crate::theme::ACCENT_HI.linear_multiply(0.45),
                    ))
                    .rounding(Rounding::same(14.0))
                    .inner_margin(egui::Margin {
                        left: 8.0,
                        right: 14.0,
                        top: 7.0,
                        bottom: 7.0,
                    })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            crate::widgets::logo(ui, 24.0);
                            ui.add_space(8.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(
                                    egui::Label::new(
                                        RichText::new("Mining reward")
                                            .size(10.0)
                                            .color(crate::theme::ACCENT_HI),
                                    )
                                    .extend(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(compact_amount(reward.amount))
                                            .size(15.0)
                                            .strong()
                                            .color(Color32::WHITE),
                                    )
                                    .extend(),
                                );
                            });
                        });
                    });
            });
    }
}

/// `+6 NIGHT`, `+6.125 NIGHT`: the full eight decimals made the chip wrap.
fn compact_amount(darks: u64) -> String {
    let full = Amount(darks).decimal_string();
    let trimmed = full.trim_end_matches('0').trim_end_matches('.');
    format!("+{trimmed} {}", nightfall_types::TICKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mined(id: &str) -> HistoryEntry {
        HistoryEntry {
            direction: Direction::Mined,
            amount: 100_000_000,
            fee: 0,
            memo: String::new(),
            height: Some(42),
            txid: id.into(),
            timestamp: 1,
            spent_commits: vec![],
            raw: None,
            quarantined: false,
        }
    }

    #[test]
    fn one_reward_once_through_repeat_frames_reorg_rescan_and_unlock() {
        let mut rewards = Rewards::default();
        rewards.observe(&[mined("old")], false);
        assert!(!rewards.advance(0.0));
        rewards.observe(&[mined("old"), mined("new")], false);
        assert!(rewards.advance(1.0));
        for time in [1.1, 1.5, 2.0, 3.0] {
            rewards.observe(&[mined("new")], false);
            assert!(!rewards.advance(time));
        }
        rewards.observe(&[], false);
        rewards.conceal();
        rewards.observe(&[], false);
        rewards.observe(&[mined("new")], false);
        assert!(!rewards.advance(5.0));
        rewards.observe(&[mined("later")], false);
        assert!(rewards.advance(6.0));
    }

    #[test]
    fn reward_amount_is_short_enough_for_one_line() {
        assert_eq!(compact_amount(600_000_000), "+6 NIGHT");
        assert_eq!(compact_amount(612_500_000), "+6.125 NIGHT");
        assert_eq!(compact_amount(1), "+0.00000001 NIGHT");
    }

    #[test]
    fn catchup_is_silent_and_multiple_live_rewards_are_sequenced_once_each() {
        let mut rewards = Rewards::default();
        rewards.observe(&[mined("history")], true);
        rewards.observe(&[mined("one"), mined("two")], false);
        assert!(rewards.advance(0.0));
        assert!(!rewards.advance(0.1));
        assert!(rewards.advance(3.0));
        assert!(!rewards.advance(6.0));
        assert!(rewards.active.is_none());
        rewards.conceal();
        assert!(rewards.pending.is_empty());
    }
}
