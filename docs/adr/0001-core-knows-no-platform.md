# The core knows no platform

Locators, auto-waiting, `expect`, snapshots, recording and traces live in `raywright-core`,
which depends on no platform crate; everything that touches the OS is behind the traits in
`driver` and implemented in `raywright-windows` (and the mock). CI fails if the core ever
depends on `windows`.

This costs a layer of `dyn` traits and an in-memory driver to maintain. It buys tests of
all of Raywright's behaviour on any OS, on paused tokio time, and a Linux or macOS driver
that is one new crate rather than a rewrite.
