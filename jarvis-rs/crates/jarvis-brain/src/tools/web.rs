use super::{ToolDef, ToolOutput, s, safe};
use crate::Ctx;
use anyhow::{Context, Result, ensure};
use jarvis_core::settings::{NewsSources, Units};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

fn client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("Jarvis desktop assistant")
        .build()?)
}

#[derive(Serialize, Deserialize)]
struct Place {
    name: String,
    latitude: f64,
    longitude: f64,
    admin1: Option<String>,
    country: Option<String>,
}
async fn place(http: &Client, city: &str) -> Result<Place> {
    ensure!(
        !city.trim().is_empty(),
        "Choose a city or set a home city in Settings"
    );
    let mut parts = city.split(',').map(str::trim);
    let name = parts.next().unwrap_or_default();
    let hint = parts.collect::<Vec<_>>().join(" ").to_lowercase();
    #[derive(Deserialize)]
    struct Places {
        #[serde(default)]
        results: Vec<Place>,
    }
    let url = Url::parse_with_params(
        "https://geocoding-api.open-meteo.com/v1/search",
        [("name", name), ("count", "10"), ("language", "en")],
    )?;
    let mut places = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<Places>()
        .await?
        .results;
    ensure!(!places.is_empty(), "Could not find {city}");
    let index = places
        .iter()
        .position(|p| {
            format!(
                "{} {}",
                p.admin1.as_deref().unwrap_or_default(),
                p.country.as_deref().unwrap_or_default()
            )
            .to_lowercase()
            .contains(&hint)
        })
        .unwrap_or(0);
    Ok(places.swap_remove(index))
}

async fn xweather(http: &Client, place: &Place, days: &str, id: &str, key: &str) -> Result<Value> {
    let fetch = |endpoint: &'static str| async move {
        let mut url = Url::parse(&format!(
            "https://data.api.xweather.com/{endpoint}/{:.4},{:.4}",
            place.latitude, place.longitude
        ))?;
        url.query_pairs_mut()
            .append_pair("client_id", id)
            .append_pair("client_secret", key);
        if endpoint == "forecasts" {
            url.query_pairs_mut()
                .append_pair("filter", "day")
                .append_pair("limit", days);
        }
        let response = http
            .get(url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?
            .error_for_status()
            .map_err(reqwest::Error::without_url)?
            .json::<Value>()
            .await
            .map_err(reqwest::Error::without_url)?;
        if response["error"]["code"] == "warn_no_data" {
            return Ok::<Value, anyhow::Error>(json!([]));
        }
        ensure!(
            response["success"] == true,
            "Weather provider returned an error"
        );
        Ok(response["response"].clone())
    };
    let (current, daily, alerts, air) = tokio::join!(
        fetch("conditions"),
        fetch("forecasts"),
        fetch("alerts"),
        fetch("airquality")
    );
    let current = current?;
    ensure!(
        current[0]["periods"][0].is_object(),
        "No current conditions"
    );
    let daily = daily?;
    Ok(
        json!({"source":"Xweather","place":place,"current":current[0]["periods"][0],"daily":daily[0]["periods"],"alerts":alerts.ok(),"air_quality":air.ok()}),
    )
}

async fn weather(ctx: Ctx, v: Value) -> Result<ToolOutput> {
    let home = ctx.settings.get().location;
    let http = client()?;
    let place = place(&http, s(&v, "location").unwrap_or(&home.city)).await?;
    let days = v["days"].as_u64().unwrap_or(3).to_string();
    let imperial = s(&v, "units")
        .map(|u| u == "imperial")
        .unwrap_or(home.units == Units::Imperial);
    if let (Some(id), Some(key)) = (
        jarvis_core::paths::secret("XWEATHER_CLIENT_ID"),
        jarvis_core::paths::secret("XWEATHER_CLIENT_SECRET"),
    ) {
        if let Ok(mut data) = xweather(&http, &place, &days, &id, &key).await {
            data["preferred_units"] = json!(if imperial { "imperial" } else { "metric" });
            return Ok(data.to_string().into());
        }
    }
    let mut url = Url::parse("https://api.open-meteo.com/v1/forecast")?;
    url.query_pairs_mut().extend_pairs([
        ("latitude",place.latitude.to_string().as_str()),("longitude",place.longitude.to_string().as_str()),("timezone","auto"),("forecast_days",&days),
        ("current","temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m,precipitation"),
        ("daily","weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max,sunrise,sunset")]);
    if imperial {
        url.query_pairs_mut().extend_pairs([
            ("temperature_unit", "fahrenheit"),
            ("wind_speed_unit", "mph"),
            ("precipitation_unit", "inch"),
        ]);
    }
    let forecast = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    Ok(
        json!({"source":"Open-Meteo","place":place,"forecast":forecast,"alerts_available":false})
            .to_string()
            .into(),
    )
}

#[derive(Deserialize, Default)]
struct Channel {
    #[serde(default)]
    item: Vec<Item>,
}
#[derive(Deserialize)]
struct Feed {
    channel: Channel,
}
#[derive(Deserialize)]
struct Item {
    title: String,
    #[serde(default)]
    source: Source,
    #[serde(default, rename = "pubDate")]
    date: String,
}
#[derive(Deserialize, Default)]
struct Source {
    #[serde(rename = "$text", default)]
    name: String,
}
impl Item {
    fn wire(&self) -> bool {
        matches!(
            self.source.name.to_ascii_lowercase().as_str(),
            "reuters"
                | "ap news"
                | "the associated press"
                | "associated press"
                | "afp"
                | "agence france-presse"
        )
    }
}
async fn feed(
    http: &Client,
    region: &str,
    path: &[&str],
    query: Option<&str>,
) -> Result<Vec<Item>> {
    let mut url = Url::parse("https://news.google.com/rss")?;
    url.path_segments_mut().unwrap().extend(path);
    url.query_pairs_mut()
        .append_pair("hl", &format!("en-{region}"))
        .append_pair("gl", region)
        .append_pair("ceid", &format!("{region}:en"));
    if let Some(q) = query {
        url.query_pairs_mut().append_pair("q", q);
    }
    let xml = http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(quick_xml::de::from_str::<Feed>(&xml)?.channel.item)
}
async fn news(ctx: Ctx, v: Value) -> Result<ToolOutput> {
    let home = ctx.settings.get().location;
    let region = home.news_region.to_uppercase();
    let http = client()?;
    let scope = s(&v, "scope").unwrap_or("top");
    let count = v["count"].as_u64().unwrap_or(6) as usize;
    let mut items = if let Some(query) = s(&v, "query") {
        feed(
            &http,
            &region,
            &["search"],
            Some(&format!("{query} when:3d")),
        )
        .await?
    } else if let Some(topic) = s(&v, "topic") {
        feed(
            &http,
            &region,
            &["headlines", "section", "topic", &topic.to_uppercase()],
            None,
        )
        .await?
    } else if scope == "local" || scope == "state" {
        ensure!(
            !home.city.trim().is_empty(),
            "Set your home city for local or state news"
        );
        if scope == "local" {
            feed(
                &http,
                &region,
                &[
                    "headlines",
                    "section",
                    "geo",
                    home.city.split(',').next().unwrap().trim(),
                ],
                None,
            )
            .await?
        } else {
            let state = place(&http, &home.city)
                .await?
                .admin1
                .context("No region found for city")?;
            let ambiguous = matches!(state.as_str(), "Washington" | "Georgia" | "New York");
            let search = if ambiguous {
                format!("{state} state")
            } else {
                state.clone()
            };
            let mut items = if ambiguous {
                feed(
                    &http,
                    &region,
                    &["search"],
                    Some(&format!("\"{search}\" when:2d")),
                )
                .await?
            } else {
                feed(
                    &http,
                    &region,
                    &["headlines", "section", "geo", &state],
                    None,
                )
                .await?
            };
            if items.len() < 3 {
                items = feed(
                    &http,
                    &region,
                    &["search"],
                    Some(&format!("\"{search}\" when:1d")),
                )
                .await?;
            }
            items
        }
    } else {
        let mut items = match scope {
            "world" => {
                feed(
                    &http,
                    &region,
                    &["headlines", "section", "topic", "WORLD"],
                    None,
                )
                .await?
            }
            "national" => {
                feed(
                    &http,
                    &region,
                    &["headlines", "section", "topic", "NATION"],
                    None,
                )
                .await?
            }
            _ => feed(&http, &region, &[], None).await?,
        };
        if s(&v, "sources")
            .map(|s| s == "neutral")
            .unwrap_or(home.news_sources == NewsSources::Neutral)
        {
            items.retain(Item::wire);
            if items.len() < count {
                let q = format!(
                    "(site:reuters.com OR site:apnews.com OR site:afp.com) when:1d {}",
                    if scope == "world" { "world" } else { "" }
                );
                for item in feed(&http, &region, &["search"], Some(&q))
                    .await?
                    .into_iter()
                    .filter(Item::wire)
                {
                    if !items.iter().any(|i| i.title == item.title) {
                        items.push(item);
                    }
                }
            }
        }
        items
    };
    items.retain(|i| !i.title.trim().is_empty());
    let lines = items
        .into_iter()
        .take(count)
        .map(|i| {
            format!(
                "{} ({}, {})",
                i.title.trim_end_matches(&format!(" - {}", i.source.name)),
                i.source.name,
                i.date
            )
        })
        .collect::<Vec<_>>();
    Ok(if lines.is_empty() {
        "No headlines found.".into()
    } else {
        lines.join("\n")
    }
    .into())
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "get_weather",
            description: "Get live conditions and a daily forecast. Mention available weather alerts.",
            risk: safe,
            summary: |_| "check the weather".into(),
            schema: || json!({"type":"object","properties":{"location":{"type":"string"},"days":{"type":"integer","minimum":1,"maximum":7},"units":{"enum":["metric","imperial"],"type":"string"}}}),
            run: |ctx, v| Box::pin(weather(ctx, v)),
        },
        ToolDef {
            name: "get_news",
            description: "Read current headlines by region, topic or query. Neutral sources select wire services.",
            risk: safe,
            summary: |_| "check the news".into(),
            schema: || json!({"type":"object","properties":{"scope":{"type":"string","enum":["top","local","state","national","world"]},"topic":{"type":"string","enum":["business","technology","science","health","sports","entertainment"]},"query":{"type":"string"},"sources":{"type":"string","enum":["neutral","all"]},"count":{"type":"integer","minimum":1,"maximum":15}}}),
            run: |ctx, v| Box::pin(news(ctx, v)),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rss_preserves_headlines_and_wire_sources() {
        let feed=quick_xml::de::from_str::<Feed>(r#"<rss><channel><item><title>Rain &amp; snow - Reuters</title><source url="https://example.org">Reuters</source><pubDate>Friday</pubDate></item></channel></rss>"#).unwrap();
        assert_eq!(feed.channel.item[0].title, "Rain & snow - Reuters");
        assert!(feed.channel.item[0].wire());
    }
    #[tokio::test]
    #[ignore = "queries live weather and news providers; use isolated JARVIS_DATA_DIR"]
    async fn live_weather_and_news() {
        let ctx = Ctx::new(jarvis_core::Bus::new()).unwrap();
        let (forecast, headlines) = tokio::join!(
            weather(
                ctx.clone(),
                json!({"location":"London, United Kingdom","days":1})
            ),
            news(ctx, json!({"topic":"science","count":2}))
        );
        assert!(forecast.unwrap().log_text().contains("London"));
        assert_eq!(headlines.unwrap().log_text().lines().count(), 2);
    }
}
