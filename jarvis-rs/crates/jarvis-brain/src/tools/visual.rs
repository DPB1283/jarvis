//! `show_visual`: draw something inside the orb itself. The orb expands to show a network of connected ideas, a
//! timeline, or an image, then shrinks back. The tool only validates and tidies the data and emits an event;
//! the drawing is done by the HUD (see `Event::Visual`).

use super::{ToolDef, s, safe};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use jarvis_core::Event;
use serde_json::{Value, json};
use std::collections::HashSet;

fn clip(text: &str, max: usize) -> String {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max { text } else { format!("{}…", text.chars().take(max - 1).collect::<String>()) }
}

/// A network: nodes with labels, links between them. Bad links are dropped rather than failing the whole picture.
pub(crate) fn web_data(input: &Value) -> Result<Value> {
    let mut nodes = Vec::new();
    let mut ids = HashSet::new();
    for node in input["nodes"].as_array().context("A web needs a list of nodes")?.iter().take(40) {
        let label = clip(node["label"].as_str().unwrap_or_default(), 48);
        if label.is_empty() {
            continue;
        }
        let id = node["id"].as_str().map(|i| clip(i, 40)).filter(|i| !i.is_empty()).unwrap_or_else(|| label.clone());
        if !ids.insert(id.clone()) {
            continue;
        }
        nodes.push(json!({"id": id, "label": label, "group": node["group"].as_str().map(|g| clip(g, 24))}));
    }
    ensure!(nodes.len() >= 2, "A web needs at least two nodes");
    let mut links = Vec::new();
    for link in input["links"].as_array().into_iter().flatten().take(120) {
        let (from, to) = (link["from"].as_str().unwrap_or_default(), link["to"].as_str().unwrap_or_default());
        if from != to && ids.contains(from) && ids.contains(to) {
            links.push(json!({"from": from, "to": to, "label": link["label"].as_str().map(|l| clip(l, 40))}));
        }
    }
    let center = input["center"].as_str().filter(|c| ids.contains(*c));
    Ok(json!({"nodes": nodes, "links": links, "center": center}))
}

pub(crate) fn timeline_data(input: &Value) -> Result<Value> {
    let mut events = Vec::new();
    for event in input["events"].as_array().context("A timeline needs a list of events")?.iter().take(24) {
        let (when, title) = (clip(event["when"].as_str().unwrap_or_default(), 30), clip(event["title"].as_str().unwrap_or_default(), 60));
        if when.is_empty() || title.is_empty() {
            continue;
        }
        events.push(json!({"when": when, "title": title, "detail": event["detail"].as_str().map(|d| clip(d, 220))}));
    }
    ensure!(events.len() >= 2, "A timeline needs at least two events");
    Ok(json!({"events": events}))
}

/// When the open orb display closes by itself (None when nothing is showing). A click on the HUD's close button
/// isn't reported back, so a quick "expand" right after that is simply ignored by the HUD.
static OPEN_UNTIL: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// The HUD closed the display itself (its x, Escape, a click outside, or its timer).
pub fn mark_closed() {
    if let Ok(mut until) = OPEN_UNTIL.lock() {
        *until = None;
    }
}

pub(crate) fn visual_open() -> bool {
    OPEN_UNTIL.lock().map(|until| until.is_some_and(|t| t > std::time::Instant::now())).unwrap_or(false)
}

/// A spoken or typed "expand" / "shrink" aimed at the orb display, answered without asking the AI.
/// Some(true) = make it bigger, Some(false) = back to normal size.
pub(crate) fn size_command(text: &str) -> Option<bool> {
    let words = crate::dismiss::normalise(text);
    let words = words
        .trim_start_matches("hey ")
        .trim_start_matches("jarvis ")
        .trim_start_matches("ultron ")
        .trim_start_matches("please ")
        .trim_end_matches(" please");
    let thing = r"(?: (?:it|that|this|the (?:orb|ball|globe|display|picture|graph|web|timeline|image|visual|hologram)))?";
    let bigger = regex::Regex::new(&format!(r"^(?:expand{thing}(?: more| further| even more)?|(?:make|get){thing} (?:even )?(?:bigger|larger|huge)|(?:even )?bigger|enlarge{thing}|zoom in|blow{thing} up|full ?screen{thing}|go full ?screen)$")).ok()?;
    let smaller = regex::Regex::new(&format!(r"^(?:shrink{thing}(?: back)?|(?:make|get){thing} (?:smaller|normal(?: size)?)|smaller|zoom out|collapse{thing}|normal size|(?:go )?back to normal size)$")).ok()?;
    if bigger.is_match(words) {
        Some(true)
    } else if smaller.is_match(words) {
        Some(false)
    } else {
        None
    }
}

/// Block fetching from this machine or the local network (the model must not be able to probe them).
fn public_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && !crate::provider::is_local(url)
}

fn mime_for(path: &str) -> Option<&'static str> {
    match path.rsplit('.').next()?.to_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

async fn image_data(input: &Value) -> Result<Value> {
    const LIMIT: usize = 5 * 1024 * 1024;
    let (bytes, mime) = if let Some(url) = s(input, "image_url") {
        ensure!(public_url(url), "That address is not a public web link");
        let response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()?
            .get(url)
            .send()
            .await?
            .error_for_status()?;
        let mime = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim().to_lowercase())
            .filter(|m| ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(&m.as_str()))
            .context("That link is not a PNG, JPEG, GIF or WebP image")?;
        let bytes = response.bytes().await?;
        (bytes.to_vec(), mime)
    } else if let Some(path) = s(input, "image_path") {
        let resolved = super::system::resolve_path(path);
        let mime = mime_for(path).context("Only PNG, JPEG, GIF and WebP images can be shown")?.to_owned();
        (std::fs::read(&resolved).with_context(|| format!("Could not read {}", resolved.display()))?, mime)
    } else {
        bail!("Give image_url or image_path");
    };
    ensure!(bytes.len() <= LIMIT, "That image is larger than 5 MB");
    ensure!(!bytes.is_empty(), "That image is empty");
    Ok(json!({"src": format!("data:{mime};base64,{}", STANDARD.encode(&bytes)), "caption": s(input, "caption").map(|c| clip(c, 140))}))
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "show_visual",
            description: "Draw something INSIDE the orb: it expands, shows it, then shrinks back. kind web: a network of connected ideas (nodes with id and label, links between node ids), good for 'how are these related' or explaining connections. kind timeline: events in order (when, title, optional detail). kind image: a picture from image_url or image_path with a caption. Keep labels short. Use it when a picture explains better than words; still say the key point out loud.",
            schema: || json!({"type":"object","required":["kind","title"],"properties":{
                "kind":{"type":"string","enum":["web","timeline","image"]},
                "title":{"type":"string","description":"A short heading shown above the picture"},
                "nodes":{"type":"array","maxItems":40,"items":{"type":"object","required":["label"],"properties":{"id":{"type":"string"},"label":{"type":"string"},"group":{"type":"string"}}}},
                "links":{"type":"array","maxItems":120,"items":{"type":"object","required":["from","to"],"properties":{"from":{"type":"string"},"to":{"type":"string"},"label":{"type":"string"}}}},
                "center":{"type":"string","description":"web: the id of the main node"},
                "events":{"type":"array","maxItems":24,"items":{"type":"object","required":["when","title"],"properties":{"when":{"type":"string"},"title":{"type":"string"},"detail":{"type":"string"}}}},
                "image_url":{"type":"string"},"image_path":{"type":"string"},"caption":{"type":"string"},
                "seconds":{"type":"integer","minimum":5,"maximum":300,"description":"How long to keep it open (default 60)"},
                "large":{"type":"boolean","description":"Open it at the big size straight away (when the user asks for it big)"}}}),
            risk: safe,
            summary: |v| format!("show a {} in the orb", s(v, "kind").unwrap_or("visual")),
            run: |ctx, v| {
                Box::pin(async move {
                    let kind = s(&v, "kind").unwrap_or_default().to_owned();
                    let data = match kind.as_str() {
                        "web" => web_data(&v)?,
                        "timeline" => timeline_data(&v)?,
                        "image" => image_data(&v).await?,
                        other => bail!("Unknown visual kind {other}"),
                    };
                    let title = clip(s(&v, "title").unwrap_or("Jarvis"), 60);
                    let ttl = v["seconds"].as_u64().map(|n| n.clamp(5, 300) as u32).or(Some(60));
                    if let Ok(mut until) = OPEN_UNTIL.lock() {
                        *until = Some(std::time::Instant::now() + std::time::Duration::from_secs(u64::from(ttl.unwrap_or(60))));
                    }
                    ctx.bus.emit(Event::Visual { kind: kind.clone(), title, data, ttl_secs: ttl });
                    if v["large"].as_bool() == Some(true) {
                        ctx.bus.emit(Event::VisualSize { large: true });
                    }
                    Ok(format!("The {kind} is showing in the orb.").into())
                })
            },
        },
        ToolDef {
            name: "hide_visual",
            description: "Close whatever the orb is showing and shrink it back to normal.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "close the orb display".into(),
            run: |ctx, _| {
                Box::pin(async move {
                    if let Ok(mut until) = OPEN_UNTIL.lock() {
                        *until = None;
                    }
                    ctx.bus.emit(Event::VisualClose {});
                    Ok("Closed.".into())
                })
            },
        },
        ToolDef {
            name: "resize_visual",
            description: "Make the open orb display (the 3D web, timeline or image) bigger, or back to its normal size. 'Expand it', 'make it bigger', 'full screen' → large true; 'shrink it', 'smaller' → large false.",
            schema: || json!({"type":"object","required":["large"],"properties":{"large":{"type":"boolean"}}}),
            risk: safe,
            summary: |v| if v["large"].as_bool() == Some(false) { "shrink the orb display".into() } else { "expand the orb display".into() },
            run: |ctx, v| {
                Box::pin(async move {
                    if !visual_open() {
                        return Ok("Nothing is showing in the orb right now. Use show_visual first (it takes large: true).".into());
                    }
                    let large = v["large"].as_bool().unwrap_or(true);
                    ctx.bus.emit(Event::VisualSize { large });
                    Ok(if large { "Expanded." } else { "Back to normal size." }.into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webs_are_tidied_and_bad_links_dropped() {
        let data = web_data(&json!({
            "nodes": [{"id":"a","label":"Sun"},{"id":"b","label":"  Earth   and   Moon "},{"id":"a","label":"Duplicate"},{"label":""},{"label":"Mars"}],
            "links": [{"from":"a","to":"b","label":"light"},{"from":"a","to":"ghost"},{"from":"b","to":"b"},{"from":"a","to":"Mars"}],
            "center": "a"
        })).unwrap();
        let nodes = data["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), 3, "duplicate and empty nodes are skipped: {nodes:?}");
        assert_eq!(nodes[1]["label"], "Earth and Moon");
        assert_eq!(nodes[2]["id"], "Mars", "a node without an id uses its label");
        assert_eq!(data["links"].as_array().unwrap().len(), 2, "unknown targets and self-links are dropped");
        assert_eq!(data["center"], "a");
        assert!(web_data(&json!({"nodes":[{"label":"Only one"}]})).is_err());
        assert!(web_data(&json!({})).is_err());
    }

    #[test]
    fn timelines_need_two_complete_events() {
        let data = timeline_data(&json!({"events":[{"when":"1969","title":"Moon landing"},{"when":"","title":"No date"},{"when":"1981","title":"First shuttle","detail":"Columbia"}]})).unwrap();
        assert_eq!(data["events"].as_array().unwrap().len(), 2);
        assert!(timeline_data(&json!({"events":[{"when":"1969","title":"Alone"}]})).is_err());
    }

    #[test]
    fn expand_and_shrink_commands() {
        for phrase in ["Expand", "expand it", "Jarvis, expand the orb.", "make it bigger", "Make it even bigger please", "bigger", "even bigger", "full screen", "Full-screen it", "expand it more", "Ultron, enlarge the hologram", "zoom in", "blow it up"] {
            assert_eq!(size_command(phrase), Some(true), "{phrase}");
        }
        for phrase in ["shrink it", "Shrink it back", "make it smaller", "smaller", "back to normal size", "collapse the globe", "zoom out"] {
            assert_eq!(size_command(phrase), Some(false), "{phrase}");
        }
        for phrase in ["expand on that idea", "make the text bigger in word", "what is bigger, the sun or jupiter", "shrink my photo", "zoom in on google maps"] {
            assert_eq!(size_command(phrase), None, "{phrase}");
        }
    }

    #[test]
    fn long_text_is_clipped_on_a_word_boundary_of_characters() {
        assert_eq!(clip("a b  c", 10), "a b c");
        assert_eq!(clip("héllo wörld", 6).chars().count(), 6);
    }

    #[test]
    fn only_public_web_images_are_fetched() {
        assert!(public_url("https://example.com/cat.png"));
        for blocked in ["http://localhost:8080/x.png", "http://192.168.1.5/x.png", "file:///C:/secret.png", "ftp://example.com/x.png", "http://127.0.0.1/x.png"] {
            assert!(!public_url(blocked), "{blocked}");
        }
        assert_eq!(mime_for("a.JPG"), Some("image/jpeg"));
        assert_eq!(mime_for("a.exe"), None);
    }

    #[tokio::test]
    async fn a_local_image_becomes_a_data_uri() {
        let dir = std::env::temp_dir().join(format!("jarvis-visual-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dot.png");
        std::fs::write(&path, [0x89, b'P', b'N', b'G', 1, 2, 3]).unwrap();
        let data = image_data(&json!({"image_path": path.to_string_lossy(), "caption": "A dot"})).await.unwrap();
        assert!(data["src"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(data["caption"], "A dot");
        assert!(image_data(&json!({"image_path": dir.join("notes.txt").to_string_lossy()})).await.is_err());
        assert!(image_data(&json!({})).await.is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
