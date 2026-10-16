//! A trace as one self-contained HTML page (pictures inlined), light and dark.

use base64::Engine;

use crate::trace::Trace;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn img(files: &[(String, Vec<u8>)], name: &Option<String>, alt: &str) -> String {
    let Some(name) = name else { return String::new() };
    match files.iter().find(|(n, _)| n == name) {
        Some((_, bytes)) => format!("<figure><img alt=\"{alt}\" src=\"data:image/png;base64,{}\"><figcaption>{alt}</figcaption></figure>", base64::engine::general_purpose::STANDARD.encode(bytes)),
        None => String::new(),
    }
}

pub fn render(trace: &Trace, files: &[(String, Vec<u8>)]) -> String {
    let status = match trace.passed {
        Some(true) => "<span class=\"ok\">passed</span>",
        Some(false) => "<span class=\"bad\">failed</span>",
        None => "<span>in progress</span>",
    };
    let mut rows = String::new();
    for (i, e) in trace.events.iter().enumerate() {
        let args = if e.args.is_null() { String::new() } else { format!("<code>{}</code>", esc(&e.args.to_string())) };
        let err = e.error.as_ref().map(|m| format!("<pre class=\"bad\">{}</pre>", esc(m))).unwrap_or_default();
        rows.push_str(&format!(
            "<details{open}><summary><b>{n}.</b> <span class=\"act\">{action}</span> <code>{sel}</code> <span class=\"dim\">{win} · {start:.0} ms · {dur:.0} ms</span>{flag}</summary>{args}{err}<div class=\"shots\">{before}{after}</div></details>\n",
            open = if e.error.is_some() { " open" } else { "" },
            n = i + 1,
            action = esc(&e.action),
            sel = esc(e.selector.as_deref().unwrap_or("")),
            win = esc(e.window.as_deref().unwrap_or("")),
            start = e.start_ms,
            dur = e.duration_ms,
            flag = if e.error.is_some() { " <span class=\"bad\">failed</span>" } else { "" },
            before = img(files, &e.before, "before"),
            after = img(files, &e.after, "after"),
        ));
    }
    let attachments: String = trace.attachments.iter().map(|(title, f)| img(files, &Some(f.clone()), &esc(title))).collect();
    format!(
        r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Raywright trace</title>
<style>
:root {{ --bg:#fafafa; --fg:#1c1c1e; --dim:#6b6b70; --line:#e2e2e6; --ok:#1a7f37; --bad:#c62828; --card:#fff; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme="light"]) {{ --bg:#141416; --fg:#ececf0; --dim:#9a9aa2; --line:#2c2c30; --ok:#4cc26a; --bad:#ff6b6b; --card:#1c1c1f; }} }}
:root[data-theme="dark"] {{ --bg:#141416; --fg:#ececf0; --dim:#9a9aa2; --line:#2c2c30; --ok:#4cc26a; --bad:#ff6b6b; --card:#1c1c1f; }}
body {{ background:var(--bg); color:var(--fg); font:14px/1.5 system-ui, sans-serif; margin:0 auto; max-width:1100px; padding:16px; }}
h1 {{ font-size:20px; margin:0 0 4px; }} .dim {{ color:var(--dim); }} .ok {{ color:var(--ok); }} .bad {{ color:var(--bad); }}
details {{ background:var(--card); border:1px solid var(--line); border-radius:8px; margin:8px 0; padding:8px 12px; }}
summary {{ cursor:pointer; }} .act {{ font-weight:600; }} code {{ font-size:13px; word-break:break-all; }}
pre {{ white-space:pre-wrap; }} .shots {{ display:flex; flex-wrap:wrap; gap:12px; }} figure {{ margin:8px 0; max-width:100%; }}
img {{ max-width:100%; border:1px solid var(--line); border-radius:4px; }} figcaption {{ color:var(--dim); font-size:12px; }}
</style></head><body>
<h1>{test}</h1><p class="dim">{app} · {started} · {status} · {count} actions</p>
{rows}
{attach}
</body></html>"#,
        test = esc(&trace.test),
        app = esc(&trace.app),
        started = esc(&trace.started),
        count = trace.events.len(),
        attach = if attachments.is_empty() { String::new() } else { format!("<h2>Attachments</h2>{attachments}") },
    )
}
