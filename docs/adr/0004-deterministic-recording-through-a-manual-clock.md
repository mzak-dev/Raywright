# Deterministic recordings through a manual clock

Pictures taken on a timer depend on how fast the machine is, so a recording on CI never
matches one made locally. With a bridge that supports `manual-clock`, Raywright stops the
app's animation time and moves it in fixed steps, waiting for each frame before taking its
picture. The same app state then gives the same frames, which makes keyframes usable as
snapshots.

Apps must route animation time through one clock for this to work, and should keep it in
whole milliseconds so rounding doesn't differ between runs. Timer-based recording remains
the default because it needs nothing from the app.
