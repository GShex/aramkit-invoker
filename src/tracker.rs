/// A champion that was finalized in a Mayhem champion-select session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub game_id: i64,
    pub champion_id: i64,
}

/// Tracks the champion-select lifecycle and decides when a champion is final.
///
/// ARAM: Mayhem has no explicit lock-in button. The final champion is only
/// known once the client leaves champion select. The watcher normally waits
/// for `InProgress`, but may also fire during `GameStart` after a short grace
/// period so the page is ready while the game is still loading.
#[derive(Debug, Clone, Default)]
pub struct ChampionTracker {
    active: Option<ActiveChampionSelect>,
}

#[derive(Debug, Clone)]
struct ActiveChampionSelect {
    game_id: i64,
    champion_id: i64,
    mayhem: Option<bool>,
}

impl ChampionTracker {
    /// Starts tracking a new champion-select session, discarding stale state.
    pub fn begin_champ_select(&mut self, game_id: i64, mayhem: Option<bool>) {
        self.active = Some(ActiveChampionSelect {
            game_id,
            champion_id: 0,
            mayhem,
        });
    }

    /// Updates the champion and queue information while champion select runs.
    ///
    /// `champion_id` may be `None` until the client has assigned a champion.
    /// Passing `None` for `mayhem` means "unknown", not "not Mayhem".
    pub fn update_champ_select(
        &mut self,
        game_id: i64,
        mayhem: Option<bool>,
        champion_id: Option<i64>,
    ) {
        if self.active.is_none() {
            self.begin_champ_select(game_id, mayhem);
        }

        let Some(active) = self.active.as_mut() else {
            return;
        };

        if active.game_id == 0 && game_id != 0 {
            active.game_id = game_id;
        }
        if mayhem.is_some() {
            active.mayhem = mayhem;
        }
        if let Some(champion_id) = champion_id.filter(|id| *id > 0) {
            active.champion_id = champion_id;
        }
    }

    /// Whether the pending champion-select session belongs to `game_id`.
    ///
    /// Returns `false` when either ID is unknown, which forces the watcher to
    /// start a fresh session after reconnecting.
    pub fn tracks_game(&self, game_id: i64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.game_id != 0 && active.game_id == game_id)
    }

    pub fn has_different_game(&self, game_id: i64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.game_id != 0 && game_id != 0 && active.game_id != game_id)
    }

    /// Feeds a non-champion-select gameflow phase.
    ///
    /// Returns a trigger exactly once when the game is actually in progress
    /// after a champion-select session in ARAM: Mayhem. `GameStart` only
    /// updates queue information and keeps the pending pick alive.
    pub fn on_phase(&mut self, phase: &str, game_id: i64, mayhem: Option<bool>) -> Option<Trigger> {
        if self.has_different_game(game_id) || is_terminal_phase(phase) {
            self.active = None;
            return None;
        }
        let active = self.active.as_mut()?;
        if let Some(mayhem) = mayhem {
            active.mayhem = Some(mayhem);
        }

        if phase == "InProgress" {
            return self.take_ready_trigger(game_id);
        }

        None
    }

    /// Fires the pending pick during a sustained `GameStart` phase.
    ///
    /// The watcher calls this after a short grace period so a `FailedToLaunch`
    /// transition can still cancel the page.
    pub fn fire_if_ready(&mut self, game_id: i64) -> Option<Trigger> {
        self.take_ready_trigger(game_id)
    }

    fn take_ready_trigger(&mut self, game_id: i64) -> Option<Trigger> {
        let active = self.active.as_ref()?;
        let same_game = active.game_id == 0 || game_id == 0 || active.game_id == game_id;
        let ready = active.mayhem == Some(true) && active.champion_id > 0 && same_game;
        if !ready {
            return None;
        }

        let trigger = Trigger {
            game_id: if active.game_id != 0 {
                active.game_id
            } else {
                game_id
            },
            champion_id: active.champion_id,
        };
        self.active = None;
        Some(trigger)
    }
}

fn is_terminal_phase(phase: &str) -> bool {
    matches!(
        phase,
        "None"
            | "Lobby"
            | "Matchmaking"
            | "ReadyCheck"
            | "EndOfGame"
            | "WaitingForStats"
            | "PreEndOfGame"
            | "TerminatedInError"
            | "FailedToLaunch"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mayhem_tracker(champion_id: i64) -> ChampionTracker {
        let mut tracker = ChampionTracker::default();
        tracker.begin_champ_select(42, Some(true));
        tracker.update_champ_select(42, Some(true), Some(champion_id));
        tracker
    }

    fn trigger(game_id: i64, champion_id: i64) -> Option<Trigger> {
        Some(Trigger {
            game_id,
            champion_id,
        })
    }

    #[test]
    fn triggers_when_game_is_in_progress() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(
            tracker.on_phase("InProgress", 42, Some(true)),
            trigger(42, 103)
        );
    }

    #[test]
    fn keeps_the_pick_until_queue_information_arrives() {
        let mut tracker = ChampionTracker::default();
        tracker.update_champ_select(0, None, Some(103));
        assert!(!tracker.has_different_game(42));
        tracker.update_champ_select(42, None, None);

        assert_eq!(tracker.on_phase("InProgress", 42, None), None);
        assert_eq!(
            tracker.on_phase("InProgress", 42, Some(true)),
            trigger(42, 103)
        );
        assert_eq!(tracker.on_phase("InProgress", 42, Some(true)), None);
    }

    #[test]
    fn game_start_keeps_the_pending_champion_for_the_grace_path() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(tracker.on_phase("GameStart", 42, Some(true)), None);
        assert_eq!(tracker.fire_if_ready(42), trigger(42, 103));
    }

    #[test]
    fn game_start_then_in_progress_fires_once() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(tracker.on_phase("GameStart", 42, Some(true)), None);
        assert_eq!(
            tracker.on_phase("InProgress", 42, Some(true)),
            trigger(42, 103)
        );
        assert_eq!(tracker.fire_if_ready(42), None);
    }

    #[test]
    fn failed_to_launch_cancels_the_pending_champion() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(tracker.on_phase("GameStart", 42, Some(true)), None);
        assert_eq!(tracker.on_phase("FailedToLaunch", 42, Some(true)), None);
        assert_eq!(tracker.on_phase("InProgress", 42, Some(true)), None);
        assert_eq!(tracker.fire_if_ready(42), None);
    }

    #[test]
    fn does_not_trigger_for_other_queues() {
        let mut tracker = ChampionTracker::default();
        tracker.begin_champ_select(42, Some(false));
        tracker.update_champ_select(42, Some(false), Some(103));

        assert_eq!(tracker.on_phase("InProgress", 42, Some(false)), None);
    }

    #[test]
    fn does_not_trigger_when_champion_is_unknown() {
        let mut tracker = ChampionTracker::default();
        tracker.begin_champ_select(42, Some(true));

        assert_eq!(tracker.on_phase("GameStart", 42, Some(true)), None);
        assert_eq!(tracker.fire_if_ready(42), None);
        assert_eq!(tracker.on_phase("InProgress", 42, Some(true)), None);
    }

    #[test]
    fn dodge_clears_pending_champion() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(tracker.on_phase("Lobby", 42, Some(true)), None);
        assert_eq!(tracker.on_phase("InProgress", 42, Some(true)), None);
        assert_eq!(tracker.fire_if_ready(42), None);
    }

    #[test]
    fn fires_only_once_for_a_game() {
        let mut tracker = mayhem_tracker(103);

        assert!(tracker.on_phase("InProgress", 42, Some(true)).is_some());
        assert_eq!(tracker.on_phase("InProgress", 42, Some(true)), None);
    }

    #[test]
    fn latest_champion_wins_after_bench_swap() {
        let mut tracker = mayhem_tracker(103);
        tracker.update_champ_select(42, Some(true), Some(99));

        assert_eq!(
            tracker.on_phase("InProgress", 42, Some(true)),
            trigger(42, 99)
        );
    }

    #[test]
    fn a_new_champion_select_starts_fresh() {
        let mut tracker = mayhem_tracker(103);
        assert!(tracker.on_phase("InProgress", 42, Some(true)).is_some());

        tracker.begin_champ_select(43, Some(true));
        tracker.update_champ_select(43, Some(true), Some(99));
        assert_eq!(
            tracker.on_phase("InProgress", 43, Some(true)),
            trigger(43, 99)
        );
    }

    #[test]
    fn queue_known_only_at_game_start_still_fires_after_grace() {
        let mut tracker = ChampionTracker::default();
        tracker.begin_champ_select(42, None);
        tracker.update_champ_select(42, None, Some(103));

        assert_eq!(tracker.on_phase("GameStart", 42, Some(true)), None);
        assert_eq!(tracker.fire_if_ready(42), trigger(42, 103));
    }

    #[test]
    fn different_game_id_does_not_fire() {
        let mut tracker = mayhem_tracker(103);

        assert_eq!(tracker.on_phase("InProgress", 999, Some(true)), None);
    }

    #[test]
    fn tracks_game_only_for_a_known_matching_game() {
        let mut tracker = ChampionTracker::default();
        assert!(!tracker.tracks_game(42));

        tracker.begin_champ_select(42, Some(true));
        assert!(tracker.tracks_game(42));
        assert!(!tracker.tracks_game(43));
        assert!(!tracker.tracks_game(0));
    }
}
