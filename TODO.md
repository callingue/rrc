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
