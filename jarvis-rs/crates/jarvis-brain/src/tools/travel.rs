//! Travel times and maps through Mapbox (the user's own free access token, set in Settings → API keys).
//!
//! The home address is private: it lives in the encrypted key store, is never part of what Claude is sent, and
//! is only used here as the start of a route when the user says "from home" (or gives no start). Results name it
//! "home", never the street address.

use super::{ToolDef, ToolOutput, s, safe};
use anyhow::{Context, Result, bail};
use base64::{Engine, engine::general_purpose::STANDARD};
use jarvis_core::Event;
use jarvis_core::settings::Units;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

const TOKEN: &str = "MAPBOX_ACCESS_TOKEN";
const ADDRESS: &str = "HOME_ADDRESS";

fn client() -> Result<Client> {
    Ok(Client::builder().timeout(Duration::from_secs(15)).user_agent("Jarvis desktop assistant").build()?)
}

fn token() -> Result<String> {
    jarvis_core::paths::secret(TOKEN).context("Mapbox is not set up. The user can paste a free Mapbox access token in Settings, API keys.")
}

#[derive(Debug, Clone)]
struct Place {
    name: String,
    lon: f64,
    lat: f64,
}

/// Escape everything but the unreserved URL characters (Mapbox wants the route polyline inside the path).
fn escape(text: &str) -> String {
    text.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

async fn get_json(http: &Client, url: &str) -> Result<Value> {
    let response = http.get(url).send().await?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let reason = body["message"].as_str().unwrap_or("request failed");
        bail!("Mapbox said: {reason} ({status})");
    }
    Ok(body)
}

/// Find a place by name or address, preferring results near `near` (so "Costco" means the one by home).
async fn geocode(http: &Client, token: &str, query: &str, near: Option<&Place>) -> Result<Place> {
    let mut url = format!(
        "https://api.mapbox.com/search/geocode/v6/forward?q={}&limit=1&access_token={}",
        escape(query),
        escape(token)
    );
    if let Some(p) = near {
        url.push_str(&format!("&proximity={},{}", p.lon, p.lat));
    }
    let body = get_json(http, &url).await?;
    let feature = body["features"].get(0).with_context(|| format!("Mapbox could not find \"{query}\""))?;
    let coordinates = &feature["geometry"]["coordinates"];
    let (lon, lat) = (coordinates[0].as_f64(), coordinates[1].as_f64());
    let (Some(lon), Some(lat)) = (lon, lat) else { bail!("Mapbox returned no position for \"{query}\"") };
    let name = feature["properties"]["name_preferred"]
        .as_str()
        .or_else(|| feature["properties"]["name"].as_str())
        .unwrap_or(query)
        .to_owned();
    Ok(Place { name, lon, lat })
}

fn is_home(text: &str) -> bool {
    matches!(
        text.trim().to_lowercase().as_str(),
        "" | "home" | "my house" | "my home" | "house" | "here" | "my place" | "my address" | "from home"
    )
}

/// "1 hour 5 minutes", "12 minutes", "under a minute".
pub(crate) fn spoken_duration(seconds: f64) -> String {
    let minutes = (seconds / 60.0).round() as i64;
    let (h, m) = (minutes / 60, minutes % 60);
    let part = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match (h, m) {
        (0, 0) => "under a minute".into(),
        (0, m) => part(m, "minute"),
        (h, 0) => part(h, "hour"),
        (h, m) => format!("{} {}", part(h, "hour"), part(m, "minute")),
    }
}

pub(crate) fn spoken_distance(meters: f64, units: Units) -> String {
    match units {
        Units::Imperial => {
            let miles = meters / 1609.344;
            if miles < 0.2 { format!("{} feet", (meters * 3.28084 / 10.0).round() as i64 * 10) } else { format!("{miles:.1} miles") }
        }
        Units::Metric => {
            if meters < 1000.0 { format!("{} metres", (meters / 10.0).round() as i64 * 10) } else { format!("{:.1} kilometres", meters / 1000.0) }
        }
    }
}

fn profile(mode: &str) -> Result<(&'static str, &'static str)> {
    Ok(match mode {
        "" | "driving" | "drive" | "car" => ("driving-traffic", "by car"),
        "walking" | "walk" | "foot" => ("walking", "on foot"),
        "cycling" | "bike" | "bicycle" => ("cycling", "by bike"),
        other => bail!("Unknown mode {other}; use driving, walking or cycling"),
    })
}

fn static_style() -> &'static str {
    "dark-v11"
}

/// A static map image (PNG, base64) of a route or a single place.
async fn static_map(http: &Client, token: &str, overlays: &str, view: &str) -> Result<String> {
    let url = format!(
        "https://api.mapbox.com/styles/v1/mapbox/{}/static/{overlays}/{view}/600x360@2x?padding=56&logo=false&attribution=false&access_token={}",
        static_style(),
        escape(token)
    );
    let response = http.get(&url).send().await?;
    if !response.status().is_success() {
        bail!("Mapbox could not draw the map ({})", response.status());
    }
    Ok(STANDARD.encode(response.bytes().await?))
}

fn show(ctx: &crate::Ctx, title: String, caption: String, image: String) {
    ctx.bus.emit(Event::Map { title, caption, image });
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "travel_time",
            description: "How long it takes to get somewhere, with live traffic for driving, via Mapbox. origin defaults to the user's saved home address (it is private: never ask for it, say it or show it; just leave origin out or pass \"home\"). Set show_map true to show the route on a map next to the orb, when the user asks for the route, directions or a map.",
            schema: || json!({"type":"object","required":["destination"],"properties":{
                "destination":{"type":"string","description":"A place or address, e.g. Costco, JFK airport, 10 Downing Street London"},
                "origin":{"type":"string","description":"Start. Omit or \"home\" for the saved home address; otherwise a place or address."},
                "mode":{"type":"string","enum":["driving","walking","cycling"]},
                "show_map":{"type":"boolean","description":"Show the route on a map on screen"}}}),
            risk: safe,
            summary: |v| format!("work out travel time to {}", v["destination"].as_str().unwrap_or("a place")),
            run: |ctx, v| {
                Box::pin(async move {
                    let token = token()?;
                    let http = client()?;
                    let (profile, by) = profile(s(&v, "mode").unwrap_or("driving"))?;
                    let destination_text = s(&v, "destination").context("Missing destination")?.trim().to_owned();
                    let origin_text = s(&v, "origin").unwrap_or("").trim().to_owned();
                    let from_home = is_home(&origin_text);
                    let home = if from_home {
                        let address = jarvis_core::paths::secret(ADDRESS)
                            .context("No home address saved. The user can add one in Settings, Home (it stays private).")?;
                        Some(geocode(&http, &token, &address, None).await?)
                    } else {
                        None
                    };
                    let start = match &home {
                        Some(h) => h.clone(),
                        None => geocode(&http, &token, &origin_text, None).await?,
                    };
                    let end = geocode(&http, &token, &destination_text, Some(&start)).await?;
                    let url = format!(
                        "https://api.mapbox.com/directions/v5/mapbox/{profile}/{},{};{},{}?overview=simplified&geometries=polyline&access_token={}",
                        start.lon, start.lat, end.lon, end.lat, escape(&token)
                    );
                    let body = get_json(&http, &url).await?;
                    let route = body["routes"].get(0).context("No route found between those places")?;
                    let (seconds, meters) = (route["duration"].as_f64().unwrap_or(0.0), route["distance"].as_f64().unwrap_or(0.0));
                    let settings = ctx.settings.get();
                    let from = if from_home { "home".to_owned() } else { start.name.clone() };
                    let arrive = (chrono::Local::now() + chrono::Duration::seconds(seconds as i64)).format("%-I:%M %p").to_string();
                    let mut text = format!(
                        "{} {by} from {from} to {}, {}. Leaving now, arrival about {arrive}.",
                        spoken_duration(seconds),
                        end.name,
                        spoken_distance(meters, settings.location.units)
                    );
                    if v["show_map"].as_bool().unwrap_or(false) && settings.location.show_maps {
                        let geometry = route["geometry"].as_str().unwrap_or_default();
                        let overlays = format!(
                            "path-6+2dd4bf-0.9({}),pin-s-a+2dd4bf({},{}),pin-s-b+ff5f5f({},{})",
                            escape(geometry), start.lon, start.lat, end.lon, end.lat
                        );
                        match static_map(&http, &token, &overlays, "auto").await {
                            Ok(image) => {
                                show(&ctx, format!("{from} to {}", end.name), format!("{} {by}, {}", spoken_duration(seconds), spoken_distance(meters, settings.location.units)), image);
                                text.push_str(" The map is on screen.");
                            }
                            Err(error) => text.push_str(&format!(" (I could not draw the map: {error})")),
                        }
                    }
                    Ok(text.into())
                })
            },
        },
        ToolDef {
            name: "show_map",
            description: "Show a map of a place next to the orb (via Mapbox), for \"show me where X is\" or \"pull up a map of X\".",
            schema: || json!({"type":"object","required":["place"],"properties":{"place":{"type":"string","description":"A place or address"}}}),
            risk: safe,
            summary: |v| format!("show a map of {}", v["place"].as_str().unwrap_or("a place")),
            run: |ctx, v| {
                Box::pin(async move {
                    ensure_maps_enabled(&ctx)?;
                    let token = token()?;
                    let http = client()?;
                    let query = s(&v, "place").context("Missing place")?.trim().to_owned();
                    let home = jarvis_core::paths::secret(ADDRESS);
                    let near = match home {
                        Some(address) => geocode(&http, &token, &address, None).await.ok(),
                        None => None,
                    };
                    let place = geocode(&http, &token, &query, near.as_ref()).await?;
                    let overlays = format!("pin-l+2dd4bf({},{})", place.lon, place.lat);
                    let image = static_map(&http, &token, &overlays, &format!("{},{},14", place.lon, place.lat)).await?;
                    show(&ctx, place.name.clone(), String::new(), image);
                    Ok(ToolOutput::Text(format!("Showing a map of {} on screen.", place.name)))
                })
            },
        },
    ]
}

fn ensure_maps_enabled(ctx: &crate::Ctx) -> Result<()> {
    if !ctx.settings.get().location.show_maps {
        bail!("Maps are turned off in Settings");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_and_distances_read_naturally() {
        assert_eq!(spoken_duration(20.0), "under a minute");
        assert_eq!(spoken_duration(60.0), "1 minute");
        assert_eq!(spoken_duration(12.0 * 60.0 + 20.0), "12 minutes");
        assert_eq!(spoken_duration(3600.0), "1 hour");
        assert_eq!(spoken_duration(3900.0), "1 hour 5 minutes");
        assert_eq!(spoken_duration(2.0 * 3600.0 + 120.0), "2 hours 2 minutes");
        assert_eq!(spoken_distance(6400.0, Units::Metric), "6.4 kilometres");
        assert_eq!(spoken_distance(6400.0, Units::Imperial), "4.0 miles");
        assert_eq!(spoken_distance(300.0, Units::Metric), "300 metres");
    }

    #[test]
    fn route_polylines_survive_the_url() {
        let polyline = "_p~iF~ps|U_ulLnnqC_mqNvxq`@";
        let escaped = escape(polyline);
        assert!(!escaped.contains('`') && !escaped.contains('~') || escaped.contains("~"), "tilde is unreserved");
        assert!(escaped.chars().all(|c| c.is_ascii_alphanumeric() || "-_.~%".contains(c)), "{escaped}");
        assert_eq!(escape("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn home_is_recognised_in_many_wordings() {
        for home in ["", "home", "Home", "my house", "here", "my place"] {
            assert!(is_home(home), "{home}");
        }
        for other in ["Costco", "work", "10 Downing Street"] {
            assert!(!is_home(other), "{other}");
        }
        assert!(profile("flying").is_err());
        assert_eq!(profile("walk").unwrap().0, "walking");
    }
}
