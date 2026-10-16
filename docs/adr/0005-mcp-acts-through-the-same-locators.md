# The MCP server acts through the same locators as tests

`raywright-mcp` has no automation of its own: each ref in a snapshot is stored as the
shortest selector that matches only that element, and every tool runs through `Locator`
with auto-waiting. An agent's session therefore behaves like a test and turns into one
(`generate_test`) without translation.

Because it drives a real desktop, it is locked down by default: only allowed executables
and profiles, no attach, input posted to the app's windows rather than the user's mouse,
files only in its output directory.
