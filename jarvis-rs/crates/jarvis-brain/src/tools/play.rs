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
            description: "Start a game against the user: chess, connect_four, sea_battle, checkers or dots_and_boxes. Opens a little page in their browser where they play by clicking or by saying their moves (use game_move); you are the opponent (the page runs the engine). difficulty beginner, easy, medium (default), hard or expert. For chess, color is the color the USER plays (white, black or random). Twenty questions needs no page: use twenty_questions.",
            schema: || json!({"type":"object","required":["game"],"properties":{
                "game":{"type":"string","enum":["chess","connect_four","sea_battle","checkers","dots_and_boxes"]},
                "difficulty":{"type":"string","enum":["beginner","easy","medium","hard","expert"]},
                "color":{"type":"string","enum":["white","black","random"],"description":"Chess: the color the user plays"},
                "first":{"type":"string","enum":["you","jarvis"],"description":"Connect Four: who starts (you = the user)"}}}),
            risk: safe,
            summary: |v| format!("start a game of {}", s(v, "game").unwrap_or("chess").replace('_', " ")),
            run: |_, v| {
                Box::pin(async move {
                    let server = crate::games::server().await?;
                    crate::games::reset();
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
                        "sea_battle" | "seabattle" => (server.url("seabattle", &format!("level={lvl}")), format!("sea battle at {difficulty} difficulty")),
                        "checkers" => (server.url("checkers", &format!("level={lvl}")), format!("checkers at {difficulty} difficulty")),
                        "dots_and_boxes" | "dots" => (server.url("dots", &format!("level={lvl}")), format!("dots and boxes at {difficulty} difficulty")),
                        other => bail!("I don't know the game {other}"),
                    };
                    open_in_browser(&url)?;
                    Ok(format!("Opened {what}. The user plays on the page by clicking, or tells you their move and you pass it to game_move. Use game_status to see the position when they ask what you think, want a hint, or the game ends.").into())
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
        ToolDef {
            name: "game_move",
            description: "Play the move the user just said in the open game page, exactly as they said it: sea battle \"fire at B7\", checkers \"c3 to d4\" (a jump: \"c3 e5 c7\"), dots and boxes \"a1 a2\" (the line between two neighbouring dots), Connect Four \"column 4\". \"new game\" and \"hint\" work too. Returns whether it worked, what happened, and your reply move, so you can react in character.",
            schema: || json!({"type":"object","required":["move"],"properties":{"move":{"type":"string","description":"The user's move in their words"}}}),
            risk: safe,
            summary: |v| format!("play {}", s(v, "move").unwrap_or("the move")),
            run: |_, v| {
                Box::pin(async move {
                    let said = s(&v, "move").unwrap_or("").trim().to_lowercase();
                    if said.is_empty() {
                        bail!("Say the move, for example \"fire at B7\"");
                    }
                    let Some(id) = crate::games::send_command(&said) else {
                        return Ok("No game page is open. Use play_game to start one.".into());
                    };
                    // Wait for the page to try the move (it checks every 0.6 s), then for your reply move.
                    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                    let outcome = loop {
                        if let Some(state) = crate::games::last_state().filter(|s| s["lastCommand"]["id"].as_u64() == Some(id)) {
                            break state;
                        }
                        if tokio::time::Instant::now() > deadline {
                            return Ok("The game page didn't respond. Is it still open in the browser? (Moves can also be made by clicking.)".into());
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                    };
                    let command = &outcome["lastCommand"];
                    if command["ok"].as_bool() != Some(true) {
                        return Ok(format!("That move didn't work: {}", command["error"].as_str().unwrap_or("not a legal move")).into());
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1400)).await; // the page plays your reply
                    let state = crate::games::last_state().unwrap_or(outcome);
                    Ok(format!("Done. {}", summary(&state)).into())
                })
            },
        },
        ToolDef {
            name: "twenty_questions",
            description: "Keep score in a game of twenty questions (voice only, no page). start: mode you_guess (you secretly pick something and the user asks yes/no questions; give secret and a category) or i_guess (the user thinks of something and you ask). record: after each question, with the question and the answer, returns how many are left. status: the secret (if you picked it) and the questions so far. end: finish (with outcome). Answer the user's questions truthfully about your secret; never change it mid-game.",
            schema: || json!({"type":"object","required":["action"],"properties":{
                "action":{"type":"string","enum":["start","record","status","end"]},
                "mode":{"type":"string","enum":["you_guess","i_guess"],"description":"you_guess = the USER guesses your secret"},
                "secret":{"type":"string","description":"start, you_guess mode: what you picked"},
                "category":{"type":"string","description":"start: e.g. animal, object, famous person, place"},
                "question":{"type":"string"},"answer":{"type":"string"},
                "outcome":{"type":"string","description":"end: who won, e.g. 'user guessed it in 14'"}}}),
            risk: safe,
            summary: |v| match s(v, "action") { Some("start") => "start twenty questions".into(), Some("end") => "end twenty questions".into(), _ => "keep score in twenty questions".into() },
            run: |_, v| Box::pin(async move { twenty_questions(&v).map(Into::into) }),
        },
    ]
}

/// A short, readable summary of a game state for the model, after a spoken move.
fn summary(state: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(shots) = state["lastShots"].as_array() {
        parts.push(shots.iter().take(2).filter_map(Value::as_str).collect::<Vec<_>>().join(" "));
    }
    if let Some(moves) = state["moves"].as_array().or(state["history"].as_array()) {
        parts.push(format!("Recent moves: {}", moves.iter().rev().take(4).rev().map(|m| m.as_str().map(str::to_owned).unwrap_or_else(|| m.to_string())).collect::<Vec<_>>().join(", ")));
    }
    if let Some(score) = state.get("score").filter(|s| !s.is_null()) {
        parts.push(format!("Score: {score}"));
    }
    parts.push(format!("Status: {}, turn: {}.", state["status"].as_str().unwrap_or("playing"), state["turn"].as_str().unwrap_or("?")));
    parts.retain(|p| !p.trim().is_empty());
    parts.join(" ")
}

#[derive(Default)]
struct TwentyQuestions {
    mode: String,
    secret: String,
    category: String,
    asked: Vec<(String, String)>,
}

static TWENTY: std::sync::Mutex<Option<TwentyQuestions>> = std::sync::Mutex::new(None);

fn twenty_questions(v: &Value) -> Result<String> {
    let mut game = TWENTY.lock().unwrap();
    match s(v, "action") {
        Some("start") => {
            let mode = s(v, "mode").unwrap_or("you_guess").to_owned();
            let secret = s(v, "secret").unwrap_or("").trim().to_owned();
            if mode == "you_guess" && secret.is_empty() {
                bail!("Pick your secret first and pass it as secret (the user won't see it)");
            }
            let category = s(v, "category").unwrap_or("anything").to_owned();
            *game = Some(TwentyQuestions { mode: mode.clone(), secret, category: category.clone(), asked: Vec::new() });
            Ok(if mode == "you_guess" {
                format!("Started. Your secret ({category}) is locked in; the user has 20 yes/no questions. Tell them the category and invite the first question.")
            } else {
                format!("Started. The user is thinking of something ({category}); ask your first yes/no question. You have 20.")
            })
        }
        Some("record") => {
            let state = game.as_mut().context("No game of twenty questions is running; start one first")?;
            state.asked.push((s(v, "question").unwrap_or("?").to_owned(), s(v, "answer").unwrap_or("?").to_owned()));
            let left = 20usize.saturating_sub(state.asked.len());
            Ok(match (left, state.mode.as_str()) {
                (0, "you_guess") => format!("That was question 20, the last one. If they haven't got it, reveal the secret: {}.", state.secret),
                (0, _) => "That was question 20. Make your final guess now; if it's wrong, concede gracefully.".into(),
                (n, _) => format!("Question {} recorded; {n} left.", state.asked.len()),
            })
        }
        Some("status") => {
            let state = game.as_ref().context("No game of twenty questions is running")?;
            let asked = state.asked.iter().enumerate().map(|(i, (q, a))| format!("{}. {q} → {a}", i + 1)).collect::<Vec<_>>().join("\n");
            Ok(format!(
                "Mode: {}. Category: {}.{} Questions used: {} of 20.\n{asked}",
                if state.mode == "you_guess" { "the user guesses your secret" } else { "you guess the user's thing" },
                state.category,
                if state.secret.is_empty() { String::new() } else { format!(" Your secret: {}.", state.secret) },
                state.asked.len()
            ))
        }
        Some("end") => {
            let state = game.take().context("No game of twenty questions is running")?;
            Ok(format!("Ended after {} questions ({}).{}", state.asked.len(), s(v, "outcome").unwrap_or("no result given"), if state.secret.is_empty() { String::new() } else { format!(" The secret was {}.", state.secret) }))
        }
        _ => bail!("Unknown action"),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_questions_keeps_the_secret_and_the_count() {
        assert!(twenty_questions(&json!({"action":"start","mode":"you_guess"})).is_err(), "a secret is required");
        twenty_questions(&json!({"action":"start","mode":"you_guess","secret":"a lighthouse","category":"place"})).unwrap();
        for i in 1..=19 {
            let out = twenty_questions(&json!({"action":"record","question":format!("q{i}"),"answer":"no"})).unwrap();
            assert!(out.contains(&format!("{} left", 20 - i)), "{out}");
        }
        let last = twenty_questions(&json!({"action":"record","question":"is it tall?","answer":"yes"})).unwrap();
        assert!(last.contains("question 20") && last.contains("a lighthouse"), "{last}");
        let status = twenty_questions(&json!({"action":"status"})).unwrap();
        assert!(status.contains("20 of 20") && status.contains("20. is it tall? → yes"), "{status}");
        let end = twenty_questions(&json!({"action":"end","outcome":"Jarvis won"})).unwrap();
        assert!(end.contains("The secret was a lighthouse"));
        assert!(twenty_questions(&json!({"action":"record","question":"x","answer":"y"})).is_err(), "nothing running after end");
    }

    #[test]
    fn move_summaries_are_short_and_readable() {
        let sea = json!({"lastShots":["Jarvis: C4, miss.","You: B7, hit."],"status":"playing","turn":"you"});
        assert_eq!(summary(&sea), "Jarvis: C4, miss. You: B7, hit. Status: playing, turn: you.");
        let dots = json!({"moves":["a1-a2","b1-b2","c1-c2","c2-c3","d1-d2"],"score":{"you":2,"jarvis":1},"status":"playing","turn":"jarvis"});
        let text = summary(&dots);
        assert!(text.contains("b1-b2, c1-c2, c2-c3, d1-d2") && !text.contains("a1-a2") && text.contains("\"you\":2"), "{text}");
    }

    #[test]
    fn difficulty_names_map_to_levels() {
        assert_eq!([level("beginner"), level("easy"), level("medium"), level("hard"), level("expert"), level("anything")], [1, 2, 3, 4, 5, 3]);
    }
}
