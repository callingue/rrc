1. Add monitoring (metrics/dashboard for services)
2. rrc_verify from cfg crate
3. Measure LTO for the release profile: build with lto=false/thin/fat and
   codegen-units=16/1, record binary size and build time for each, then pick one
   and write the numbers next to the setting in Cargo.toml.
   Matters once binary size does (initramfs); no reason to guess before then.
   Do NOT adopt panic="abort" along with it: it removes catch_unwind, which a
   supervisor that may one day run as PID 1 will likely need.
