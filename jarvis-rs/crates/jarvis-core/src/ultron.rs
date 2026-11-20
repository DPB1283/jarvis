use serde::Deserialize;
use std::sync::LazyLock;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ultron {
    pub voice_id: String,
    pub prompt: String,
    pub enable: String,
    pub disable: String,
    pub mode: String,
}
pub static ULTRON: LazyLock<Ultron> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../jarvis/src/shared/ultron.json"))
        .expect("valid Ultron settings")
});
