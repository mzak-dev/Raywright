//! Raywright's core, end to end, on the mock driver: what a platform driver must make true.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use raywright_core::codegen::{Launch, ScriptRecorder, to_rust};
use raywright_core::element::Role;
use raywright_core::hooks::{ActionCtx, ActionHook};
use raywright_core::selector::{Found, QueryCx, SelectorEngine};
use raywright_core::*;
use raywright_mock::{MockDriver, MockElement};

struct Setup {
    rw: Raywright,
    driver: MockDriver,
    dir: tempfile::TempDir,
}

fn setup_with(edit: impl FnOnce(&mut Config)) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::default();
    c.base_dir = dir.path().to_path_buf();
    c.timeout = Dur::secs(2);
    c.expect_timeout = Dur::secs(2);
    c.snapshot.update = UpdateSnapshots::Missing;
    edit(&mut c);
    let driver = MockDriver::new();
    let rw = Raywright::builder().driver(driver.clone().into_arc()).config(c).build().unwrap();
    Setup { rw, driver, dir }
}

fn setup() -> Setup {
    setup_with(|_| {})
}

const DEMO: &str = "title=\"Raywright Demo\"";

#[tokio::test]
async fn the_mock_keeps_the_driver_contract() {
    let s = setup();
    let fx = raywright_core::conformance::demo("demo-app");
    let done = raywright_core::conformance::run(&s.rw, &fx).await.unwrap();
    assert_eq!(done.len(), 7, "{done:?}");
}

#[tokio::test]
async fn strict_mode_refuses_ambiguous_locators() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    // the Increment button (accessibility tree) and the canvas box (bridge) are both buttons
    let err = win.locator("role=button").click().await.unwrap_err();
    match &err {
        Error::Ambiguous { count, examples, .. } => {
            assert_eq!(*count, 2);
            assert!(examples.contains("Increment"), "{examples}");
        }
        other => panic!("{other}"),
    }
    assert_eq!(win.locator("role=button").count().await.unwrap(), 2);
    win.locator("role=button").first().click().await.unwrap();
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await.unwrap();
}

#[tokio::test]
async fn actions_wait_for_elements_to_appear_and_enable() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let mock = s.driver.app(app.pid().0).unwrap();
    mock.update(|st| {
        let w = &mut st.windows[0];
        w.elements.push(MockElement::new("late", Role::Button, raywright_core::Rect::new(300.0, 20.0, 60.0, 30.0)).name("Late").on_click(|s| {
            s.data.insert("late".into(), 1);
        }));
        w.element_mut("late").unwrap().enabled = false;
    });
    let m2 = mock.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        m2.update(|st| st.find_mut("late").unwrap().enabled = true);
    });
    let started = std::time::Instant::now();
    win.get_by_role("button", "Late").click().await.unwrap();
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert_eq!(mock.snapshot().data.get("late"), Some(&1));
}

#[tokio::test]
async fn timeouts_say_what_was_awaited() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    s.driver.app(app.pid().0).unwrap().update(|st| st.find_mut("increment").unwrap().enabled = false);
    let err = win.get_by_role("button", "Increment").click().timeout(Duration::from_millis(300)).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("timed out"), "{msg}");
    assert!(msg.contains("element is disabled"), "{msg}");
    let err = win.locator("text=Nope").click().timeout(Duration::from_millis(200)).await.unwrap_err();
    assert!(err.to_string().contains("no element matches"), "{err}");
    let err = win.locator("tetx=Nope").click().await.unwrap_err();
    assert!(matches!(err, Error::InvalidSelector { .. }), "{err}");
}

#[tokio::test]
async fn expectations_retry_and_negate() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    expect(win.get_by_text("Count: 0")).to_be_visible().await.unwrap();
    expect(win.get_by_text("Nothing")).to_be_hidden().await.unwrap();
    expect(win.get_by_text("Nothing")).not().to_be_visible().await.unwrap();
    expect(win.locator("role=checkbox")).not().to_be_checked().await.unwrap();
    expect(win.locator("role=textbox")).to_be_editable().await.unwrap();
    expect(win.locator("mock=*")).to_have_count(5).await.unwrap();
    expect(win.clone()).to_have_title("Raywright Demo").await.unwrap();
    expect(app.clone()).to_have_window("kind=main").await.unwrap();
    let e = expect(win.get_by_key("counter")).timeout(Duration::from_millis(200)).to_have_text("Count: 9").await.unwrap_err();
    let msg = e.to_string();
    assert!(msg.contains("expected: \"Count: 9\""), "{msg}");
    assert!(msg.contains("received: \"Count: 0\""), "{msg}");
}

#[tokio::test]
async fn fill_type_and_keyboard() {
    let s = setup_with(|c| c.input_mode = InputMode::Message);
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let tb = win.locator("role=textbox");
    tb.fill("abc").await.unwrap();
    expect(tb.clone()).to_have_value("abc").await.unwrap();
    tb.type_text("def").await.unwrap();
    expect(tb.clone()).to_have_value("abcdef").await.unwrap();
    tb.press("Backspace").await.unwrap();
    expect(tb.clone()).to_have_value("abcde").await.unwrap();
    win.get_by_role("button", "Increment").click().await.unwrap();
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await.unwrap();
    win.keyboard().press("Control+R").await.unwrap();
    expect(win.get_by_key("counter")).to_have_text("Count: 0").await.unwrap();
    // message input names its window; nothing was sent as OS input
    let inputs = s.driver.inputs();
    assert!(inputs.iter().all(|i| i.mode == InputMode::Message && i.window.is_some()));
}

#[tokio::test]
async fn chaining_filters_and_bridge_elements() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let canvas = win.locator("bridge=key=canvas");
    expect(canvas.get_by_key("canvas.clicks")).to_have_text("Clicks: 0").await.unwrap();
    expect(win.locator("bridge=[key^=canvas.]")).to_have_count(2).await.unwrap();
    expect(win.locator("bridge=*").filter_has_text("clicks")).to_have_count(2).await.unwrap();
    win.get_by_action("canvas.box").click().await.unwrap();
    expect(win.get_by_key("canvas.clicks")).to_have_text("Clicks: 1").await.unwrap();
    let state = app.bridge().await.unwrap().call("state", serde_json::Value::Null).await.unwrap();
    assert_eq!(state["clicks"], 1);
    let w = app.windows().await.unwrap();
    assert_eq!(w[0].kind.as_deref(), Some("main"));
}

#[tokio::test]
async fn native_mode_uses_the_elements_api() {
    let s = setup_with(|c| c.input_mode = InputMode::Native);
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    win.get_by_role("button", "Increment").click().await.unwrap();
    win.locator("role=checkbox").check().await.unwrap();
    expect(win.get_by_key("counter")).to_have_text("Count: 1").await.unwrap();
    expect(win.locator("role=checkbox")).to_be_checked().await.unwrap();
    assert!(s.driver.inputs().is_empty(), "no input was needed");
}

#[tokio::test]
async fn screenshots_snapshot_and_report_differences() {
    let s = setup_with(|c| c.snapshot.update = UpdateSnapshots::Missing);
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let img = win.screenshot().path(s.dir.path().join("w.png")).await.unwrap();
    assert_eq!(img.dimensions(), (400, 300));
    let part = win.get_by_role("button", "Increment").screenshot().await.unwrap();
    assert_eq!(part.dimensions(), (100, 30));

    // first run writes it (not in CI), second matches
    unsafe { std::env::remove_var("CI") };
    expect(win.clone()).to_have_screenshot("demo").await.unwrap();
    let expected = s.dir.path().join("tests/__screenshots__").join(raywright_core::snapshot::sanitize(&raywright_core::test_support::test_id())).join("demo.png");
    assert!(expected.exists(), "{}", expected.display());
    expect(win.clone()).to_have_screenshot("demo").await.unwrap();

    // a change shows up as a mismatch, with the actual and diff pictures
    win.get_by_role("button", "Increment").click().await.unwrap();
    let err = expect(win.clone()).timeout(Duration::from_millis(300)).to_have_screenshot("demo").await.unwrap_err();
    match err {
        Error::SnapshotMismatch { actual, diff, .. } => {
            assert!(actual.exists());
            assert!(diff.unwrap().exists());
        }
        other => panic!("{other}"),
    }
    // masking what changes keeps a snapshot stable
    let counter = win.get_by_key("counter");
    expect(win.clone()).to_have_screenshot("masked").mask(&counter).await.unwrap();
    win.get_by_role("button", "Increment").click().await.unwrap();
    expect(win.clone()).to_have_screenshot("masked").mask(&counter).await.unwrap();
}

#[tokio::test]
async fn manual_clock_recordings_are_deterministic() {
    let s = setup();
    let mut runs: Vec<Recording> = Vec::new();
    for _ in 0..2 {
        // a fresh app each time: the same state must give the same frames
        let app = s.rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await.unwrap();
        let win = app.window(DEMO).await.unwrap();
        let opts = RecordOptions::default().manual_clock(Duration::from_millis(50)).until_settled(Duration::from_millis(150)).max_frames(40);
        let (r, rec) = win.record_during(opts, async { win.get_by_action("canvas.box").click().mode(InputMode::Message).await }).await.unwrap();
        r.unwrap();
        expect(&rec).not().to_be_still().unwrap();
        expect(&rec).to_settle_within(Duration::from_millis(400)).unwrap();
        let canvas = win.get_by_key("canvas").state().await.unwrap().rect.unwrap();
        expect(&rec).to_change_only_within(canvas).unwrap();
        let files = rec.save_dir(s.dir.path().join("rec")).unwrap();
        assert!(files.iter().any(|f| f.ends_with("contact.png")));
        assert!(files.iter().any(|f| f.ends_with("anim.gif")));
        app.close().await.unwrap();
        runs.push(rec);
    }
    assert_eq!(runs[0].len(), runs[1].len());
    for (a, b) in runs[0].frames().iter().zip(runs[1].frames()) {
        assert_eq!(a.t, b.t);
        assert!(a.image == b.image, "frame at {:?} differs", a.t);
    }
    // 300 ms of slide in 50 ms steps, then three still steps
    assert_eq!(runs[0].analysis().last_change_ms, Some(300.0));
}

#[tokio::test]
async fn interval_and_frame_synced_recordings() {
    let s = setup();
    let app = s.rw.launch(LaunchSpec::exe("demo-app").wait_for_bridge()).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let opts = RecordOptions::default().fps(60.0).until_settled(Duration::from_millis(200)).max_duration(Duration::from_secs(3));
    let (r, rec) = win.record_during(opts.clone(), async { win.get_by_action("canvas.box").click().await }).await.unwrap();
    r.unwrap();
    assert!(rec.len() >= 5, "{} frames", rec.len());
    expect(&rec).to_settle_within(Duration::from_millis(1500)).unwrap();

    let (r, rec) = win.record_during(opts.frame_synced(), async { win.get_by_action("canvas.box").click().await }).await.unwrap();
    r.unwrap();
    assert!(rec.len() >= 3, "{} frames", rec.len());
    assert!(rec.frames().iter().skip(1).any(|f| f.app_frame.is_some()));
}

#[tokio::test]
async fn app_exit_ends_waiting_at_once() {
    let s = setup_with(|c| c.timeout = Dur::secs(20));
    let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    let mock = s.driver.app(app.pid().0).unwrap();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        mock.output().push("fatal: out of widgets");
        mock.exit(3);
    });
    let started = std::time::Instant::now();
    let err = win.locator("text=never").click().await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    let msg = err.to_string();
    assert!(msg.contains("exited with code 3") || matches!(err, Error::WindowClosed(_)), "{msg}");
}

/// A selector engine from outside: `even=` keeps every other element found so far.
struct EvenEngine;

#[async_trait]
impl SelectorEngine for EvenEngine {
    fn name(&self) -> &str {
        "even"
    }
    async fn query(&self, cx: &QueryCx<'_>, input: Option<&[Found]>, _body: &str) -> Result<Vec<Found>> {
        let list = match input {
            Some(l) => l.to_vec(),
            None => cx.all().await,
        };
        Ok(list.into_iter().step_by(2).collect())
    }
}

/// A hook from outside: counts actions.
#[derive(Default)]
struct Counter(std::sync::atomic::AtomicUsize);

#[async_trait]
impl ActionHook for Counter {
    async fn after(&self, _cx: &ActionCtx, _outcome: std::result::Result<(), &Error>) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

#[tokio::test]
async fn extensions_plug_in_without_changing_raywright() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Config::default();
    c.base_dir = dir.path().to_path_buf();
    let counter = Arc::new(Counter::default());
    let script = Arc::new(ScriptRecorder::default());
    let rw = Raywright::builder().driver(MockDriver::new().into_arc()).config(c).engine(Arc::new(EvenEngine)).hook(counter.clone()).hook(script.clone()).build().unwrap();
    let app = rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
    let win = app.window(DEMO).await.unwrap();
    expect(win.locator("mock=* >> even=")).to_have_count(3).await.unwrap();
    win.get_by_role("button", "Increment").click().await.unwrap();
    win.locator("role=textbox").fill("x").await.unwrap();
    assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    let code = to_rust("replay", &Launch::Profile("demo".into()), &script.steps());
    assert!(code.contains(".click().await?;"), "{code}");
    assert!(code.contains(".fill(\"x\").await?;"), "{code}");
}

#[tokio::test]
async fn traces_are_written_when_on() {
    let s = setup_with(|c| c.trace = TraceMode::On);
    {
        let app = s.rw.launch(LaunchSpec::exe("demo-app")).await.unwrap();
        let win = app.window(DEMO).await.unwrap();
        win.get_by_role("button", "Increment").click().await.unwrap();
        let path = app.tracer().unwrap().finish("traced", false).unwrap().unwrap();
        assert!(path.join("trace.zip").exists());
        let html = std::fs::read_to_string(path.join("report.html")).unwrap();
        assert!(html.contains("click"));
        let (trace, files) = raywright_core::trace::read_zip(&path.join("trace.zip")).unwrap();
        assert_eq!(trace.events.len(), 1);
        assert!(files.iter().any(|(n, _)| n.contains("before")));
    }
}

#[tokio::test]
async fn profiles_from_config() {
    let s = setup_with(|c| {
        c.profiles.insert("demo".into(), toml::from_str("exe = \"demo-app\"\nready = { window = \"title=\\\"Raywright Demo\\\"\", bridge = true }").unwrap());
    });
    let app = s.rw.launch_named("demo").await.unwrap();
    assert!(matches!(app.bridge_state(), raywright_core::bridge::BridgeState::Connected(_)));
    assert!(s.rw.launch_named("nope").await.is_err());
}
