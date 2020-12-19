#![feature(async_closure)]

extern crate mozaic_core;
extern crate warp;
#[macro_use]
extern crate serde;
extern crate futures;
extern crate serde_json;

use mozaic_core::client_manager::ClientHandle;
use futures::future;

mod planetwars;

use mozaic_core::{Token, GameServer, MatchCtx};

use std::convert::Infallible;
use warp::reply::{json, Json, Reply};
use warp::Filter;

use std::sync::{Arc, Mutex};

use hex::FromHex;
use rand::Rng;

#[derive(Serialize, Deserialize, Debug)]
struct MatchConfig {
    client_tokens: Vec<String>,
}

struct GameManager {
    game_server: GameServer,
}

impl GameManager {
    fn create_match(&mut self, config: MatchConfig) {
        let clients = config.client_tokens.iter().map(|token_hex| {
            let token = Token::from_hex(&token_hex).unwrap();
            self.game_server.get_client(&token)
        }).collect::<Vec<_>>();
    
        let match_ctx = self.game_server.create_match();
        tokio::spawn(run_match(clients, match_ctx));
    }
}

async fn run_match(mut clients: Vec<ClientHandle>, mut match_ctx: MatchCtx) {
    let players = clients.iter_mut().enumerate().map(|(i, client)| {
        let player_token: Token = rand::thread_rng().gen();
        match_ctx.create_player(i as u32, player_token);
        client.run_player(player_token)
    }).collect::<Vec<_>>();

    let config = planetwars::Config {
        map_file: "hex.json".to_string(),
        max_turns: 500,
    };

    future::join_all(players).await;
    let pw_match = planetwars::Planetwars::create(match_ctx, config);
    pw_match.run().await;
    println!("match done");
}

fn with_game_manager(
    game_manager: Arc<Mutex<GameManager>>,
) -> impl Clone + Filter<Extract = (Arc<Mutex<GameManager>>,), Error = Infallible>
{
    warp::any().map(move || game_manager.clone())
}

fn create_match(
    mgr: Arc<Mutex<GameManager>>,
    match_config: MatchConfig,
) -> impl Reply {
    let mut manager = mgr.lock().unwrap();
    manager.create_match(match_config);
    return "sure bro";
}

#[tokio::main]
async fn main() {
    let game_server = GameServer::new();
    // TODO: can we run these on the same port? Would that be desirable?
    tokio::spawn(game_server.run_ws_server("127.0.0.1:8080".to_string()));

    let game_manager = Arc::new(Mutex::new(GameManager { game_server }));

    let routes = warp::path("matches")
        .and(warp::post())
        .and(with_game_manager(game_manager))
        .and(warp::body::json())
        .map(create_match);

    warp::serve(routes).run(([127, 0, 0, 1], 3000)).await;
}
