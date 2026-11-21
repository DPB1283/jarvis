//! Games with Jarvis: `play_game` opens a chess or Connect Four page in the browser, `game_status` lets him see
//! the position so he can talk about it. See `crate::games`.

use super::{NoWindow, ToolDef, s, safe};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

fn level(name: &str) -> u8 {
    match name {
        "beginner" => 1,
        "easy" => 2,
        "hard" => 4,
        "expert" => 5,
        _ => 3,
    }
}

/// Open a URL in the default browser without going through a shell (the URL has & in it).
fn open_in_browser(url: &str) -> Result<()> {
    let mut command = if cfg!(windows) {
        let mut c = tokio::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = tokio::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.no_window().spawn().context("could not open the browser")?;
    Ok(())
}

pub fn tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "play_game",
            description: "Start a game against the user: chess or connect_four. Opens a little page in their browser where they play by clicking; you are the opponent (the page runs the engine). difficulty beginner, easy, medium (default), hard or expert. For chess, color is the color the USER plays (white, black or random).",
            schema: || json!({"type":"object","required":["game"],"properties":{
                "game":{"type":"string","enum":["chess","connect_four"]},
                "difficulty":{"type":"string","enum":["beginner","easy","medium","hard","expert"]},
                "color":{"type":"string","enum":["white","black","random"],"description":"Chess: the color the user plays"},
                "first":{"type":"string","enum":["you","jarvis"],"description":"Connect Four: who starts (you = the user)"}}}),
            risk: safe,
            summary: |v| format!("start a game of {}", s(v, "game").unwrap_or("chess").replace('_', " ")),
            run: |_, v| {
                Box::pin(async move {
                    let server = crate::games::server().await?;
                    let difficulty = s(&v, "difficulty").unwrap_or("medium").to_owned();
                    let lvl = level(&difficulty);
                    let (url, what) = match s(&v, "game").unwrap_or("chess") {
                        "chess" => {
                            let color = s(&v, "color").unwrap_or("white");
                            (server.url("chess", &format!("level={lvl}&color={color}")), format!("chess at {difficulty} difficulty, the user plays {color}"))
                        }
                        "connect_four" | "connect4" => {
                            let first = if s(&v, "first") == Some("jarvis") { "jarvis" } else { "you" };
                            (server.url("connect4", &format!("level={lvl}&first={first}")), format!("Connect Four at {difficulty} difficulty, {first} starts"))
                        }
                        other => bail!("I don't know the game {other}"),
                    };
                    open_in_browser(&url)?;
                    Ok(format!("Opened {what}. The user plays on the page; use game_status to see the position when they ask what you think, want a hint, or the game ends.").into())
                })
            },
        },
        ToolDef {
            name: "game_status",
            description: "The current state of the game being played with the user (chess position as FEN and move list, or the Connect Four board), whose turn it is and the result. Use it to comment on the game or suggest a move.",
            schema: || json!({"type":"object","properties":{}}),
            risk: safe,
            summary: |_| "look at the game".into(),
            run: |_, _| {
                Box::pin(async move {
                    let Some(state): Option<Value> = crate::games::last_state() else {
                        return Ok("No game has been started yet. Use play_game to start one.".into());
                    };
                    let mut text = serde_json::to_string_pretty(&state)?;
                    if text.len() > 6000 {
                        text.truncate(text.floor_char_boundary(6000));
                    }
                    Ok(text.into())
                })
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_names_map_to_levels() {
        assert_eq!([level("beginner"), level("easy"), level("medium"), level("hard"), level("expert"), level("anything")], [1, 2, 3, 4, 5, 3]);
    }
}
