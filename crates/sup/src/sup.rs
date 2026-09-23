use std::{collections::HashMap, path::Path, process::ExitStatus, time::Duration};

use anyhow::{Context, bail};
use rrc_cfg::{
    config::load_units,
    types::execspec::{ExecSpec, Kind},
};
use rrc_core::{
    service::ServiceName,
    state::{Flags, State, Status},
};
use rustix::process::{Pid, Signal, kill_process};
use tokio::{process::Command, sync::mpsc};

/// How long a `Simple` service must stay alive before we call it started.
///
/// A stand-in for a real readiness protocol (`kind = "notify"`): it only catches a
/// process that dies immediately, which is what a bad config or a missing file
/// looks like. Anything slower to fail still reports as started.
const READINESS_PROBE: Duration = Duration::from_millis(100);

/// How long a service gets to exit after `SIGTERM` before it is killed outright.
///
/// Together with `KILL_GRACE` this belongs in the unit file, next to the command
/// it applies to — a database needs longer to flush than an echo server. Constants
/// for now so that stopping works at all.
const DEFAULT_STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How long to wait for a process to disappear after `SIGKILL`, which it cannot
/// catch. Only a process stuck in the kernel (uninterruptible I/O) outlives this.
const DEFAULT_KILL_TIMEOUT: Duration = Duration::from_secs(2);

/// Something that happened to a service's process, reported by its watcher task.
#[derive(Debug)]
enum Event {
    Exited {
        name: ServiceName,
        status: ExitStatus,
    },
}

pub struct Supervisor {
    execs: HashMap<ServiceName, ExecSpec>,
    statuses: HashMap<ServiceName, Status>,
    /// Live processes. `Oneshot` services never appear here: they leave nothing
    /// running, yet are legitimately `Started`.
    pids: HashMap<ServiceName, u32>,
    /// Cloned for each watcher task; kept here so new ones can be spawned later.
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
}

impl Supervisor {
    pub fn new(execs: HashMap<ServiceName, ExecSpec>) -> Self {
        let statuses = execs
            .keys()
            .map(|name| (name.clone(), Status::default()))
            .collect();
        let (tx, rx) = mpsc::unbounded_channel();

        Self {
            execs,
            statuses,
            pids: HashMap::new(),
            tx,
            rx,
        }
    }

    /// Builds a supervisor from every `.service` file under `dir`.
    ///
    /// The graph half of each unit (`deps`, `provides`, `runlevels`) is dropped here:
    /// nothing consumes it until the dependency registry exists, and keeping a copy
    /// that no one reads would just be a second source of truth.
    pub fn from_dir(dir: &Path) -> anyhow::Result<Self> {
        let units = load_units(dir)?;
        let execs = units
            .into_iter()
            .map(|unit| (unit.service.name, unit.exec))
            .collect();

        Ok(Self::new(execs))
    }

    pub fn status(&self, name: &ServiceName) -> Option<Status> {
        self.statuses.get(name).copied()
    }

    /// Starts every known service.
    ///
    /// Ordered by name: there is no dependency graph yet, so this is a placeholder
    /// for the topological order a plan will provide later. A failure is reported and
    /// the run continues — nothing knows yet who depended on the failed service.
    pub async fn start_all(&mut self) {
        let mut order: Vec<ServiceName> = self.execs.keys().cloned().collect();
        order.sort();

        for name in order {
            if let Err(e) = self.start_one(&name).await {
                println!("{name:<16} !! {e:#}");
            }
        }
    }

    pub async fn start_one(&mut self, name: &ServiceName) -> anyhow::Result<()> {
        let spec = self
            .execs
            .get(name)
            .with_context(|| format!("unknown service {name}"))?;

        // Copy the command out before the first `set`: `spec` borrows `self.execs`,
        // and `set` needs `&mut self`.
        let kind = spec.kind;
        let (program, args) = spec.start.split();
        let program = program.clone();
        let args = args.to_vec();

        self.clear_failure(name);
        self.set(name, State::Starting);

        match kind {
            Kind::Oneshot => self.run_oneshot(name, &program, &args).await,
            Kind::Simple => self.run_simple(name, &program, &args).await,
        }
    }

    /// Runs to completion; the exit code decides whether the service came up.
    async fn run_oneshot(
        &mut self,
        name: &ServiceName,
        program: &str,
        args: &[String],
    ) -> anyhow::Result<()> {
        let exit = match Command::new(program).args(args).status().await {
            Ok(exit) => exit,
            // Without this arm a service with a bad command would sit in Starting forever.
            Err(e) => {
                self.fail(name);
                return Err(e).with_context(|| format!("spawning {program}"));
            }
        };

        if exit.success() {
            // A oneshot is Started with no process left alive: the work is done.
            self.set(name, State::Started);
            Ok(())
        } else {
            self.fail(name);
            bail!("exited with {exit}")
        }
    }

    /// Stays alive; the service is up as long as the process is.
    async fn run_simple(
        &mut self,
        name: &ServiceName,
        program: &str,
        args: &[String],
    ) -> anyhow::Result<()> {
        let mut child = match Command::new(program).args(args).kill_on_drop(true).spawn() {
            Ok(child) => child,
            Err(e) => {
                self.fail(name);
                return Err(e).with_context(|| format!("spawning {program}"));
            }
        };

        tokio::time::sleep(READINESS_PROBE).await;
        if let Ok(Some(exit)) = child.try_wait() {
            self.fail(name);
            bail!("exited immediately with {exit}");
        }

        let pid = match child.id() {
            Some(pid) => pid,
            None => {
                self.fail(name);
                bail!("child exited before its pid could be read");
            }
        };
        self.pids.insert(name.clone(), pid);

        // The child moves into its own task: awaiting it here would mean waiting for
        // the service to die. The task reports the exit through the channel instead.
        let name_for_task = name.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Ok(status) = child.wait().await {
                let _ = tx.send(Event::Exited {
                    name: name_for_task,
                    status,
                });
            }
        });

        self.set(name, State::Started);
        Ok(())
    }

    /// Stops every running service, in reverse of the order they were started in.
    pub async fn stop_all(&mut self) {
        let mut order: Vec<ServiceName> = self.execs.keys().cloned().collect();
        order.sort();
        order.reverse();

        for name in order {
            if let Err(e) = self.stop_one(&name).await {
                println!("{name:<16} !! {e:#}");
            }
        }
    }

    pub async fn stop_one(&mut self, name: &ServiceName) -> anyhow::Result<()> {
        let Some(status) = self.status(name) else {
            bail!("unknown service {name}");
        };
        if !matches!(status.state, State::Started | State::Inactive) {
            return Ok(());
        }

        let stop = self.execs.get(name).and_then(|spec| spec.stop.clone());
        self.set(name, State::Stopping);

        // A stop command is the only way to undo a oneshot, which has no process
        // left to signal — and the graceful path for daemons that offer one.
        if let Some(argv) = stop {
            let (program, args) = argv.split();
            match Command::new(program).args(args).status().await {
                Ok(exit) if !exit.success() => println!("{name:<16} !! stop command {exit}"),
                Err(e) => println!("{name:<16} !! stop command failed: {e}"),
                _ => {}
            }
        }

        if !self.pids.contains_key(name) {
            // Either a oneshot, or the stop command already brought it down.
            self.set(name, State::Stopped);
            return Ok(());
        }

        let stop_timeout = if self
            .execs
            .get(name)
            .and_then(|spec| spec.stop_timeout)
            .is_some()
        {
            self.execs
                .get(name)
                .and_then(|spec| spec.stop_timeout)
                .unwrap()
        } else {
            DEFAULT_STOP_TIMEOUT
        };

        self.signal(name, Signal::TERM);
        if self.await_exit(name, stop_timeout).await {
            return Ok(());
        }

        let waited = stop_timeout;
        let kill_timeout = if self
            .execs
            .get(name)
            .and_then(|spec| spec.kill_timeout)
            .is_some()
        {
            self.execs
                .get(name)
                .and_then(|spec| spec.kill_timeout)
                .unwrap()
        } else {
            DEFAULT_KILL_TIMEOUT
        };

        println!("{name:<16} !! did not stop in {waited:?}, sending SIGKILL");
        self.signal(name, Signal::KILL);
        if self.await_exit(name, kill_timeout).await {
            return Ok(());
        }

        bail!("still alive after SIGKILL")
    }

    /// Applies events until this service's process is gone. `false` on timeout.
    ///
    /// Events for other services are applied on the way through rather than
    /// dropped: this is the same queue `watch` reads.
    async fn await_exit(&mut self, name: &ServiceName, within: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + within;

        while self.pids.contains_key(name) {
            match tokio::time::timeout_at(deadline, self.rx.recv()).await {
                Ok(Some(event)) => self.on_event(event),
                // Channel closed, or the deadline passed.
                _ => return false,
            }
        }
        true
    }

    fn signal(&self, name: &ServiceName, signal: Signal) {
        let Some(&pid) = self.pids.get(name) else {
            return;
        };
        let Some(pid) = Pid::from_raw(pid as i32) else {
            return;
        };

        if let Err(e) = kill_process(pid, signal) {
            println!("{name:<16} !! could not send {signal:?}: {e}");
        }
    }

    /// Runs until interrupted, reacting to services that exit on their own.
    pub async fn watch(&mut self) {
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    println!("-- interrupted --");
                    return;
                }
                Some(event) = self.rx.recv() => self.on_event(event),
            }
        }
    }

    /// Waits for one process event and applies it. `false` if none arrived in time.
    ///
    /// Only tests need this: `watch` owns the queue in a running supervisor.
    #[cfg(test)]
    async fn handle_next_event(&mut self, within: Duration) -> bool {
        match tokio::time::timeout(within, self.rx.recv()).await {
            Ok(Some(event)) => {
                self.on_event(event);
                true
            }
            _ => false,
        }
    }

    fn on_event(&mut self, event: Event) {
        let Event::Exited { name, status } = event;
        self.pids.remove(&name);

        // Anything we did not ask for is a crash. `Stopped` alone cannot say that,
        // which is what the CRASHED flag is for.
        let expected = self.statuses.get(&name).map(|s| s.state) == Some(State::Stopping);
        println!("{name:<16} .. exited with {status}");
        self.set(&name, State::Stopped);

        if !expected && let Some(status) = self.statuses.get_mut(&name) {
            status.flags.insert(Flags::CRASHED);
        }
    }

    /// The only place that writes a state, so every change is checked by the FSM.
    fn set(&mut self, name: &ServiceName, next: State) {
        let Some(status) = self.statuses.get_mut(name) else {
            println!("{name:<16} !! state update for an unknown service");
            return;
        };

        match status.state.transition(next) {
            Ok(state) => {
                status.state = state;
                println!("{name:<16} -> {state:?}");
            }
            // Reaching an illegal transition means the supervisor itself is wrong,
            // so fail loudly in tests and stay on the last valid state in release.
            Err(e) => {
                debug_assert!(false, "{e}");
                println!("{name:<16} !! {e}");
            }
        }
    }

    /// A start that never came up: back to Stopped, and remember why.
    ///
    /// The two writes belong together — the state alone cannot express failure,
    /// since `Stopped` is also the ordinary resting state.
    fn fail(&mut self, name: &ServiceName) {
        self.set(name, State::Stopped);
        if let Some(status) = self.statuses.get_mut(name) {
            status.flags.insert(Flags::FAILED);
        }
    }

    /// A new attempt supersedes the result of the previous one.
    fn clear_failure(&mut self, name: &ServiceName) {
        if let Some(status) = self.statuses.get_mut(name) {
            status.flags.remove(Flags::FAILED);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rrc_cfg::types::execspec::Argv;

    fn spec(kind: Kind, start: &[&str]) -> ExecSpec {
        let argv: Vec<String> = start.iter().map(|s| s.to_string()).collect();
        ExecSpec {
            kind,
            start: Argv::try_from(argv).unwrap(),
            stop: None,
            reload: None,
            stop_timeout: Some(DEFAULT_STOP_TIMEOUT),
            kill_timeout: Some(DEFAULT_KILL_TIMEOUT),
        }
    }

    fn oneshot(start: &[&str]) -> ExecSpec {
        spec(Kind::Oneshot, start)
    }

    fn simple(start: &[&str]) -> ExecSpec {
        spec(Kind::Simple, start)
    }

    fn sup_of(name: &str, exec: ExecSpec) -> (Supervisor, ServiceName) {
        let name = ServiceName::new(name).unwrap();
        let sup = Supervisor::new(HashMap::from([(name.clone(), exec)]));
        (sup, name)
    }

    fn sup_with(name: &str, start: &[&str]) -> (Supervisor, ServiceName) {
        sup_of(name, oneshot(start))
    }

    fn unit_toml(name: &str) -> String {
        format!(
            "[service]\nname = \"{name}\"\ndesc = \"\"\nprovides = []\ndeps = []\n\
             runlevels = []\n\n[exec]\nkind = \"oneshot\"\nstart = [\"/bin/echo\"]\n"
        )
    }

    #[test]
    fn from_dir_registers_every_unit_as_stopped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.service"), unit_toml("a")).unwrap();
        std::fs::write(dir.path().join("b.service"), unit_toml("b")).unwrap();

        let sup = Supervisor::from_dir(dir.path()).unwrap();

        for name in ["a", "b"] {
            let name = ServiceName::new(name).unwrap();
            let status = sup.status(&name).expect("unit should be known");
            assert_eq!(status.state, State::Stopped);
            assert!(status.flags.is_empty());
        }
    }

    #[test]
    fn unknown_service_has_no_status() {
        let dir = tempfile::tempdir().unwrap();
        let sup = Supervisor::from_dir(dir.path()).unwrap();

        assert!(sup.status(&ServiceName::new("ghost").unwrap()).is_none());
    }

    #[tokio::test]
    async fn successful_oneshot_ends_started() {
        let (mut sup, name) = sup_with("ok", &["/bin/sh", "-c", "exit 0"]);

        sup.start_one(&name).await.unwrap();

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Started);
        assert!(!status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn failing_oneshot_ends_stopped_and_flagged() {
        let (mut sup, name) = sup_with("bad", &["/bin/sh", "-c", "exit 1"]);

        assert!(sup.start_one(&name).await.is_err());

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn missing_binary_does_not_leave_the_service_starting() {
        let (mut sup, name) = sup_with("ghost", &["/nonexistent/binary"]);

        assert!(sup.start_one(&name).await.is_err());

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::FAILED));
    }

    #[tokio::test]
    async fn start_all_runs_every_service() {
        let mut execs = HashMap::new();
        for name in ["a", "b"] {
            execs.insert(
                ServiceName::new(name).unwrap(),
                oneshot(&["/bin/sh", "-c", "exit 0"]),
            );
        }
        let mut sup = Supervisor::new(execs);

        sup.start_all().await;

        for name in ["a", "b"] {
            let status = sup.status(&ServiceName::new(name).unwrap()).unwrap();
            assert_eq!(status.state, State::Started);
        }
    }

    #[tokio::test]
    async fn simple_service_stays_started() {
        let (mut sup, name) = sup_of("daemon", simple(&["/bin/sleep", "3600"]));

        sup.start_one(&name).await.unwrap();

        assert_eq!(sup.status(&name).unwrap().state, State::Started);
        assert!(
            sup.pids.contains_key(&name),
            "a live process should be tracked"
        );
    }

    #[tokio::test]
    async fn simple_that_dies_at_once_fails_to_start() {
        let (mut sup, name) = sup_of("flaky", simple(&["/bin/sh", "-c", "exit 1"]));

        assert!(sup.start_one(&name).await.is_err());

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::FAILED));
        assert!(!sup.pids.contains_key(&name));
    }

    #[tokio::test]
    async fn a_service_dying_later_is_flagged_as_crashed() {
        // Outlives the readiness probe, so it starts, and only then exits.
        let (mut sup, name) = sup_of("crasher", simple(&["/bin/sh", "-c", "sleep 0.3; exit 1"]));

        sup.start_one(&name).await.unwrap();
        assert_eq!(sup.status(&name).unwrap().state, State::Started);

        assert!(
            sup.handle_next_event(Duration::from_secs(5)).await,
            "the watcher task should have reported the exit"
        );

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(status.flags.contains(Flags::CRASHED));
        assert!(!sup.pids.contains_key(&name));
    }

    #[tokio::test]
    async fn oneshot_is_started_without_a_live_process() {
        let (mut sup, name) = sup_of("work", oneshot(&["/bin/sh", "-c", "exit 0"]));

        sup.start_one(&name).await.unwrap();

        assert_eq!(sup.status(&name).unwrap().state, State::Started);
        assert!(
            !sup.pids.contains_key(&name),
            "a oneshot leaves nothing running"
        );
    }

    #[tokio::test]
    async fn stopping_a_running_service_is_not_a_crash() {
        let (mut sup, name) = sup_of("daemon", simple(&["/bin/sleep", "3600"]));
        sup.start_one(&name).await.unwrap();

        sup.stop_one(&name).await.unwrap();

        let status = sup.status(&name).unwrap();
        assert_eq!(status.state, State::Stopped);
        assert!(
            !status.flags.contains(Flags::CRASHED),
            "we asked for this exit"
        );
        assert!(!sup.pids.contains_key(&name));
    }

    #[tokio::test]
    async fn a_service_ignoring_sigterm_is_killed() {
        // `trap "" TERM` makes the shell ignore SIGTERM; the loop keeps it from
        // exec'ing away the trap, so only SIGKILL can end it.
        let (mut sup, name) = sup_of(
            "stubborn",
            simple(&["/bin/sh", "-c", "trap '' TERM; while :; do sleep 1; done"]),
        );
        sup.start_one(&name).await.unwrap();

        sup.stop_one(&name).await.unwrap();

        assert_eq!(sup.status(&name).unwrap().state, State::Stopped);
        assert!(!sup.pids.contains_key(&name));
    }

    #[tokio::test]
    async fn a_oneshot_is_undone_by_its_stop_command() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("undone");

        let mut exec = oneshot(&["/bin/sh", "-c", "exit 0"]);
        exec.stop = Some(
            Argv::try_from(vec![
                "/bin/sh".to_string(),
                "-c".to_string(),
                format!("touch {}", marker.display()),
            ])
            .unwrap(),
        );
        let (mut sup, name) = sup_of("work", exec);

        sup.start_one(&name).await.unwrap();
        sup.stop_one(&name).await.unwrap();

        assert_eq!(sup.status(&name).unwrap().state, State::Stopped);
        assert!(marker.exists(), "the stop command should have run");
    }

    #[tokio::test]
    async fn stopping_a_stopped_service_does_nothing() {
        let (mut sup, name) = sup_of("idle", simple(&["/bin/sleep", "3600"]));

        sup.stop_one(&name).await.unwrap();

        assert_eq!(sup.status(&name).unwrap().state, State::Stopped);
    }
}
