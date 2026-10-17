# Plugins are crates, composed at build time

Support for one kind of app (Godot first) lives in a plugin: a crate, in its own
repository, that bundles selector engines, profiles, a driver, test helpers and MCP tools
under one name. Tests add it with `Raywright::builder().plugin(..)`; a tool built on
`raywright-mcp` adds it in a three-line `main` (`raywright_mcp::cli::main(plugins)`).

Rust has no stable ABI, so loading plugins into a built binary would mean a C interface
for every trait or a process per plugin speaking JSON-RPC. Both cost more than they give
while the plugins are few and written in Rust: tools in another process couldn't share the
session's locators, refs and recordings, which is what makes an agent's session turn into
a test ([0005](0005-mcp-acts-through-the-same-locators.md)). A crate gets type checking,
semver from cargo, and no runtime plugin version to negotiate.

What helps every app stays here rather than in a plugin: the bridge driver and annotated
screenshots.
