use thiserror::Error;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum State {
    // The service has to be in one only at all times
    /// Not running. The default, resting state — no process, no dependents waiting on it.
    #[default]
    Stopped,
    /// Process is running and considered ready to do its job.
    Started,
    /// `stop()` is currently executing, on its way from Started to Stopped.
    Stopping,
    /// `start()` is currently executing, on its way from Stopped (or Inactive) to Started.
    Starting,
    /// Process is alive but not yet ready to work (e.g. still warming up, waiting
    /// on an external event to finish initialization).
    ///
    /// Dependents that strictly `need` this service must wait past it; dependents
    /// that merely `use`/`want` it are free to not block on it.
    Inactive,
}

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Flags: u8 {
        /// The last start/stop attempt failed, or a hard dependency failed to come up.
        ///
        /// Kept as an explicit flag (rather than just falling back to Stopped) so
        /// dependents don't blindly retry a service that's already known to be broken.
        const FAILED       = 1 << 0;
        /// Waiting for a service it depends on to become ready before actually
        /// starting itself.
        ///
        /// Once that dependency reaches Started, it will look up who's scheduled on
        /// it and start them — this is how starts get deferred instead of failing.
        const SCHEDULED    = 1 << 1;
        /// Marks that the current Starting/Stopping transition began from Inactive,
        /// not from a fresh Stopped state.
        ///
        /// Lets loosely-dependent services tell "resuming an already-partly-up
        /// service" apart from "starting cold", so they don't needlessly block-wait
        /// on what looks like the same Starting flag either way.
        const WAS_INACTIVE = 1 << 2;
        /// Set when the service moved directly from Started/Inactive to Stopped,
        /// i.e. the process died on its own — no stop() was ever running.
        const CRASHED      = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Status {
    pub state: State,
    pub flags: Flags,
}

pub enum Origin {
    // A service has exactly one origin at a time, set on its most recent start
    /// Service is a member of the runlevel that's currently being brought up —
    /// started as part of the normal boot/runlevel-switch sequence.
    Runlevel,
    /// Service was started automatically in response to a hardware/device event
    /// (e.g. a udev/devd handler bringing up a USB device or network interface),
    /// rather than by a runlevel or a user.
    ///
    /// This is not part of any runlevel's static list, so on a runlevel switch
    /// it must be tracked separately and re-added if the switch would otherwise
    /// stop it.
    Hotplugged,
    /// Service was started because another running or starting service declared
    /// it as a dependency (`need`), not because it belongs to the active
    /// runlevel or was started by the user directly.
    ///
    /// Like `Hotplugged`, this isn't part of any runlevel's static list — if the
    /// service(s) that pulled it in stop, this one may become unneeded and should
    /// be re-evaluated rather than assumed to stay running forever.
    Needed,
    /// Service was started directly by a user command (e.g. `rrc-service X start`),
    /// not via a runlevel, a hotplug event, or as someone else's dependency.
    ///
    /// There's no automatic reason for it to keep running or to be re-added on a
    /// runlevel switch — if the user wants it to survive, that's on the user, not
    /// on the state machine.
    Manual,
}

#[derive(Debug, Error)]
#[error("invalid state transition: {from:?} -> {to:?}")]
pub struct InvalidTransition {
    pub from: State,
    pub to: State,
}

impl State {
    pub fn transition(self, to: State) -> Result<State, InvalidTransition> {
        use State::*;

        let valid = matches!(
            (self, to),
            // launching (start() called)
            (Stopped, Starting)
            | (Inactive, Starting)

            // start() resolves
            | (Starting, Started)   // fully up
            | (Starting, Inactive)  // alive, not ready yet (warm-up)
            | (Starting, Stopped)   // failed before ever coming up

            // shutting down (stop() called)
            | (Started, Stopping)
            | (Inactive, Stopping)

            // stop() resolves
            | (Stopping, Stopped)   // fully down
            | (Stopping, Started)   // aborted, was fully up
            | (Stopping, Inactive)  // aborted, was only inactive

            // crash: process died on its own, no stop() ever ran
            | (Started, Stopped)
            | (Inactive, Stopped)
        );

        if valid {
            Ok(to)
        } else {
            Err(InvalidTransition { from: self, to })
        }
    }
}

// TODO: is_crashed() for Service struct

#[cfg(test)]
mod tests {
    use super::*;
    use State::*;

    const ALL: [State; 5] = [Stopped, Started, Stopping, Starting, Inactive];

    /// Every transition the state machine is meant to allow. The test below walks the
    /// whole 5x5 matrix against this table, so a change in `transition` that is not
    /// reflected here fails the build instead of silently widening what is legal.
    const ALLOWED: [(State, State); 12] = [
        // start() called
        (Stopped, Starting),
        (Inactive, Starting),
        // start() resolves
        (Starting, Started),
        (Starting, Inactive),
        (Starting, Stopped),
        // stop() called
        (Started, Stopping),
        (Inactive, Stopping),
        // stop() resolves
        (Stopping, Stopped),
        (Stopping, Started),
        (Stopping, Inactive),
        // the process died on its own, no stop() ever ran
        (Started, Stopped),
        (Inactive, Stopped),
    ];

    /// Applies each step in turn, panicking with the rejected pair if one is refused.
    fn walk(start: State, path: &[State]) -> State {
        path.iter().fold(start, |current, &next| {
            current
                .transition(next)
                .unwrap_or_else(|e| panic!("unexpected rejection: {e}"))
        })
    }

    #[test]
    fn matrix_matches_the_table() {
        for from in ALL {
            for to in ALL {
                let expected = ALLOWED.contains(&(from, to));
                let actual = from.transition(to).is_ok();
                assert_eq!(actual, expected, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn transition_yields_the_target_state() {
        assert_eq!(Stopped.transition(Starting).unwrap(), Starting);
    }

    #[test]
    fn no_state_transitions_to_itself() {
        for state in ALL {
            assert!(state.transition(state).is_err(), "{state:?} -> itself");
        }
    }

    #[test]
    fn normal_lifecycle_is_allowed() {
        assert_eq!(walk(Stopped, &[Starting, Started, Stopping, Stopped]), Stopped);
    }

    #[test]
    fn warm_up_lifecycle_is_allowed() {
        // Starting -> Inactive means alive but not ready yet; it may then finish coming up.
        assert_eq!(walk(Stopped, &[Starting, Inactive, Starting, Started]), Started);
    }

    #[test]
    fn crash_and_aborted_stop_are_allowed() {
        assert_eq!(walk(Started, &[Stopped]), Stopped); // died on its own
        assert_eq!(walk(Started, &[Stopping, Started]), Started); // stop aborted
    }

    #[test]
    fn starting_cannot_be_reached_from_started_or_stopping() {
        // A running service must go through stop() before it can start again.
        assert!(Started.transition(Starting).is_err());
        assert!(Stopping.transition(Starting).is_err());
    }

    #[test]
    fn rejection_names_both_states() {
        let err = Stopped.transition(Started).unwrap_err().to_string();
        assert!(err.contains("Stopped") && err.contains("Started"), "{err}");
    }

    #[test]
    fn default_status_is_stopped_without_flags() {
        let status = Status::default();
        assert_eq!(status.state, Stopped);
        assert!(status.flags.is_empty());
    }
}
