# Locators are selectors, never element handles

A `Locator` is a selector and a window, resolved again on every use; Raywright has no
element handle. Desktop element trees change under you (UI Automation elements go stale,
an app redraws its own UI every frame), and a handle that can go stale needs every caller
to handle it. Resolving each time also gives auto-waiting and strict mode one place to live.

Native element APIs (UI Automation Invoke and Value) are a shortcut an `ElementSource`
may take for an action, never the way elements are named.
