1. Add monitoring (metrics/dashboard for services)
2. rrc_verify from cfg crate
3. Measure LTO for the release profile: build with lto=false/thin/fat and
   codegen-units=16/1, record binary size and build time for each, then pick one
   and write the numbers next to the setting in Cargo.toml.
   Matters once binary size does (initramfs); no reason to guess before then.
   Do NOT adopt panic="abort" along with it: it removes catch_unwind, which a
   supervisor that may one day run as PID 1 will likely need.
4. Try cargo-mutants on rrc-core: it edits the code in small ways and reports which
   changes the tests fail to notice. The state machine and, later, the dependency
   graph are the targets — pure logic, no I/O. Slow, so run it by hand or on a
   schedule, not on every push.
5. Use proptest for the dependency graph once it exists. Properties worth asserting:
   for any generated DAG the resulting order places each service after all of its
   dependencies, and any graph containing a cycle is rejected rather than ordered.
6. Add cargo-deny to CI: licenses of dependencies, duplicate versions, and RUSTSEC
   advisories in one gate. Every entry in the ignore list needs a written reason
   explaining why the advisory is not reachable from this code.
7. Write a proper logger for the supervisor and retire the ad-hoc println! calls,
   keeping the three markers they already use: `->` a state change, `..` a reported
   event, `!!` something went wrong, with the service name padded into a column.
   Probably a custom formatter over tracing rather than hand-rolled printing —
   levels, per-service spans and RRC_LOG filtering come with it.
   Two decisions to make while doing it: timestamps counted from start rather than
   wall clock (during early boot the RTC may not be set yet), and keeping the
   services' own stdout/stderr out of this stream — those are raw bytes bound for a
   file or the journal, not lines to format.
8. Kill a service's whole process tree, not just the pid we spawned. `signal()` sends
   to one process, so anything the service forked survives the stop: a shell loop
   leaves its `sleep` behind, a daemon leaves its workers. Visible today in
   `a_service_ignoring_sigterm_is_killed`, which nextest reports as leaky.
   The real fix is cgroup v2: put each service in its own cgroup and write "1" to
   `cgroup.kill`, which the kernel documents as handling concurrent forks and being
   protected against migrations. A cheaper stopgap is a per-service process group
   (`Command::process_group(0)` at spawn, `kill_process_group` at stop) — it covers
   the common cases but a process can leave the group with setsid/setpgid.
