use async_std::task;
use broadcaster::BroadcastChannel;
use futures::prelude::*;
use futures_util::future::Either;
use futures_util::StreamExt;
use regex::Regex;
use serde::de;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::HashMap;
use tide_websockets::{Message as WSMessage, WebSocket};
use twitter_stream::Token;

#[derive(Debug, serde::Deserialize)]
pub struct RequestBody {
    topics: Vec<String>,
}

#[derive(Clone, Debug)]
struct State {
    broadcaster: BroadcastChannel<RoomMessage>,
    rooms: HashMap<String, Room>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StreamMessage {
    Tweet(Tweet),
    Other(de::IgnoredAny),
}

#[derive(Deserialize, Serialize, Clone, Debug)]
struct Tweet {
    id: u64,
    id_str: String,
    text: String,
    user: User,
    timestamp_ms: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
struct User {
    id: u64,
    screen_name: String,
    profile_image_url_https: String,
}

#[derive(Clone, Debug)]
struct Room {
    id: String,
    topics: Vec<String>,
    regex: Regex,
}

impl Room {
    pub fn should_send(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

#[derive(Clone, Debug)]
struct RoomMessage {
    room_id: String,
    tweet: Tweet,
}

async fn spawn_tracker(
    broadcaster: BroadcastChannel<RoomMessage>,
    rooms: HashMap<String, Room>,
    topics: String,
) {
    println!("topics : {}", topics);
    let token = Token::from_parts(
        std::env::var("TW_CONSUMER_KEY").expect("missing env var TW_CONSUMER_KEY"),
        std::env::var("TW_CONSUMER_SECRET").expect("missing env var TW_CONSUMER_SECRET"),
        std::env::var("TW_TOKEN").expect("missing env var TW_TOKEN"),
        std::env::var("TW_SECRET").expect("missing env var TW_SECRET"),
    );

    task::spawn(async move {
        let mut tracker = twitter_stream::Builder::new(token.as_ref());
        let mut stream = tracker.track(&topics).listen().try_flatten_stream();

        while let Some(json) = stream.next().await {
            if let Ok(StreamMessage::Tweet(tw)) = serde_json::from_str(&json.unwrap()) {
                //println!("receive a  tweet! ... , {}", tw.text);
                for (key, room) in &rooms {
                    if room.should_send(&tw.text) {
                        let msg = RoomMessage {
                            room_id: key.to_string(),
                            tweet: tw.clone(),
                        };
                        match broadcaster.send(&msg).await {
                            Ok(_) => {}
                            Err(_) => {
                                println!("Error sending to broadcaster")
                            }
                        };
                    }
                }
            }
        }
    });
}

fn get_topics(input_str: &str) -> Vec<String> {
    let temp_vec: Vec<&str> = input_str.split('\n').collect();
    let topics: Vec<String> = temp_vec.iter().map(|s| s.to_string()).collect();
    topics
}

fn get_regex(input_str: &str) -> String {
    let temp_vec: Vec<&str> = input_str.split('\n').collect();
    let topics: Vec<String> = temp_vec.iter().map(|s| format!(r"(\b{}\b)", s)).collect();
    topics.join("|")
}

#[async_std::main]
async fn main() -> Result<(), std::io::Error> {
    dotenv::dotenv().ok();

    tide::log::start();

    let broadcaster = BroadcastChannel::new();

    let nba_input = include_str!("../public/nba.txt");
    let rust_input = include_str!("../public/rust.txt");
    let rustdc_input = include_str!("../public/rustdc.txt");
    let premier_input = include_str!("../public/premier.txt");

    let nba_room = Room {
        id: "nba".to_string(),
        topics: get_topics(nba_input),
        regex: Regex::new(&get_regex(nba_input)).unwrap(),
    };
    let nba_topics_str = nba_room.topics.join(",");

    let rust_room = Room {
        id: "rust".to_string(),
        topics: get_topics(rust_input),
        regex: Regex::new(&get_regex(rust_input)).unwrap(),
    };
    let rust_topics_str = rust_room.topics.join(",");

    let rustdc_room = Room {
        id: "rustdc".to_string(),
        topics: get_topics(rustdc_input),
        regex: Regex::new(&get_regex(rustdc_input)).unwrap(),
    };
    let rustdc_topics_str = rustdc_room.topics.join(",");

    let premier_room = Room {
        id: "premier".to_string(),
        topics: get_topics(premier_input),
        regex: Regex::new(&get_regex(premier_input)).unwrap(),
    };
    let premier_topics_str = premier_room.topics.join(",");
    let mut rooms: HashMap<String, Room> = HashMap::new();
    rooms.insert("nba".to_string(), nba_room);
    rooms.insert("rust".to_string(), rust_room);
    rooms.insert("rustdc".to_string(), rustdc_room);
    rooms.insert("premier".to_string(), premier_room);

    // spawn tracker
    let topics_str = format!(
        "{},{},{},{}",
        nba_topics_str, rust_topics_str, premier_topics_str, rust_topics_str
    );
    spawn_tracker(broadcaster.clone(), rooms.clone(), topics_str).await;

    let mut app = tide::with_state(State { broadcaster, rooms });
    // serve public dir for assets
    app.at("/public").serve_dir("./public/")?;

    // index route
    app.at("/").serve_file("public/index.html")?;

    // ws route
    app.at("/ws").get(WebSocket::new(
        |req: tide::Request<State>, wsc| async move {
            let state = req.state().clone();
            let rooms = state.rooms;
            let broadcaster = state.broadcaster.clone();
            let mut combined_stream = futures_util::stream::select(
                wsc.clone().map(Either::Left),
                broadcaster.clone().map(Either::Right),
            );

            // by default we put new connections in the nba room
            let mut current_room = rooms.get("nba");

            while let Some(item) = combined_stream.next().await {
                match item {
                    Either::Left(Ok(WSMessage::Text(message))) => {
                        println!("message : {}", message);
                        current_room = rooms.get(&message);
                    }

                    Either::Right(room_message) => {
                        if let Some(room) = current_room {
                            if room.id == room_message.room_id {
                                wsc.send_json(&room_message.tweet).await?;
                            }
                        }
                    }
                    _o => {
                        return Err(tide::http::format_err!("no idea"));
                    }
                }
            }
            Ok(())
        },
    ));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    app.listen(addr).await?;

    Ok(())
}
