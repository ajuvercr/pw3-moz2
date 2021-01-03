#![feature(async_closure)]

use mozaic_core::client_manager::ClientHandle;
use futures::future;

mod planetwars;

use mozaic_core::{Token, GameServer, MatchCtx};

use std::convert::Infallible;
use warp::reply::{json,Reply,Response};
use warp::Filter;
use serde::{Serialize,Deserialize};

use std::sync::{Arc, Mutex};
use std::collections::HashMap;

use hex::FromHex;
use rand::Rng;

#[derive(Serialize, Deserialize, Debug)]
struct MatchConfig {
    client_tokens: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Player {
    name: String,
    #[serde(with = "hex")]
    token: Token,
    ready: bool,
}

impl Player {
    pub fn authorize_header(&self, authorization: Option<String>) -> bool {
        if authorization.is_none() {
            false
        } else {
            let bearer_token = authorization.unwrap().to_lowercase();
            let token_string = bearer_token.strip_prefix("bearer ");
            if token_string.is_none() {
                return false;
            }
            let token_opt = Token::from_hex(token_string.unwrap());
            if token_opt.is_err() || token_opt.unwrap() != self.token {
                false
            } else {
                true
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct StrippedPlayer {
    name: String,
    ready: bool,
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Lobby {
    id: String,
    name: String,
    public: bool,
    match_config: planetwars::Config,
    players: HashMap<String,Player>,
    #[serde(with = "hex")]
    lobby_token: Token
}

#[derive(Serialize, Deserialize, Debug)]
struct StrippedLobby {
    id: String,
    name: String,
    public: bool,
    match_config: planetwars::Config,
    players: HashMap<String,StrippedPlayer>,
}

#[derive(Serialize, Deserialize, Debug)]
struct LobbyConfig {
    name: String,
    public: bool,
    match_config: planetwars::Config,
}

impl From<LobbyConfig> for Lobby {
    fn from(config: LobbyConfig) -> Lobby {
        let id: [u8; 16] = rand::thread_rng().gen();
        Lobby {
            id: hex::encode(id),
            name: config.name,
            public: config.public,
            match_config: config.match_config,
            players: HashMap::new(),
            lobby_token: rand::thread_rng().gen(),
        }
    }
}

impl From<Lobby> for StrippedLobby {
    fn from(lobby: Lobby) -> StrippedLobby {
        StrippedLobby {
            id: lobby.id,
            name: lobby.name,
            public: lobby.public,
            match_config: lobby.match_config,
            players: lobby.players.iter().map(|(k,v)| (k.clone(),StrippedPlayer::from(v.clone()))).collect(),
        }
    }
}

impl From<Player> for StrippedPlayer {
    fn from(player: Player) -> StrippedPlayer {
        StrippedPlayer {
            name: player.name,
            ready: player.ready,
        }
    }
}

impl Lobby {
    pub fn authorize_header(&self, authorization: Option<String>) -> bool {
        if authorization.is_none() {
            false
        } else {
            let bearer_token = authorization.unwrap().to_lowercase();
            let token_string = bearer_token.strip_prefix("bearer ");
            if token_string.is_none() {
                return false;
            }
            let token_opt = Token::from_hex(token_string.unwrap());
            if token_opt.is_err() || token_opt.unwrap() != self.lobby_token {
                false
            } else {
                true
            }
        }
    }
}

struct LobbyManager {
    game_manager: Arc<Mutex<GameManager>>,
    lobbies: HashMap<String, Lobby>
}

impl LobbyManager {
    pub fn new(game_manager: Arc<Mutex<GameManager>>) -> Self {
        Self {
            game_manager,
            lobbies: HashMap::new(),
        }
    }

    pub fn create_lobby(&mut self, config: LobbyConfig) -> Lobby {
        let lobby: Lobby = config.into();
        self.lobbies.insert(lobby.id.clone(), lobby.clone());
        lobby
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
    let pw_match = planetwars::PwMatch::create(match_ctx, config);
    pw_match.run().await;
    println!("match done");
}

fn with_game_manager(
    game_manager: Arc<Mutex<GameManager>>,
) -> impl Clone + Filter<Extract = (Arc<Mutex<GameManager>>,), Error = Infallible>
{
    warp::any().map(move || game_manager.clone())
}

fn with_lobby_manager(
    lobby_manager: Arc<Mutex<LobbyManager>>,
) -> impl Clone + Filter<Extract = (Arc<Mutex<LobbyManager>>,), Error = Infallible>
{
    warp::any().map(move || lobby_manager.clone())
}

fn create_match(
    mgr: Arc<Mutex<GameManager>>,
    match_config: MatchConfig,
) -> impl Reply {
    let mut manager = mgr.lock().unwrap();
    manager.create_match(match_config);
    return "sure bro";
}

fn create_lobby(
    mgr: Arc<Mutex<LobbyManager>>,
    lobby_config: LobbyConfig,
) -> impl Reply {
    let mut manager = mgr.lock().unwrap();
    let lobby = manager.create_lobby(lobby_config);
    json(&lobby)
}

fn get_lobbies(
    mgr: Arc<Mutex<LobbyManager>>,
) -> impl Reply {
    let manager = mgr.lock().unwrap();
    return json(&manager.lobbies.values().filter_map(|lobby| {
        if lobby.public {
            Some((*lobby).clone().into())
        } else {
            None
        }
    }).collect::<Vec<StrippedLobby>>());
}

fn get_lobby_by_id(
    id: String,
    mgr: Arc<Mutex<LobbyManager>>,
) -> Response {
    let manager = mgr.lock().unwrap();
    match manager.lobbies.get(&id.to_lowercase()) {
        Some(lobby) => {
            json(&StrippedLobby::from(lobby.clone())).into_response()
        },
        None => warp::http::StatusCode::NOT_FOUND.into_response()
    }
}

fn update_lobby_by_id(
    id: String,
    mgr: Arc<Mutex<LobbyManager>>,
    authorization: Option<String>,
    lobby_conf: LobbyConfig,
) -> Response {
    let mut manager = mgr.lock().unwrap();
    match manager.lobbies.get(&id.to_lowercase()) {
        Some(lobby) => {
            if lobby.authorize_header(authorization) {
                let mut new_lobby = lobby.clone();
                new_lobby.name = lobby_conf.name;
                new_lobby.public = lobby_conf.public;
                new_lobby.match_config = lobby_conf.match_config;
                manager.lobbies.insert(new_lobby.id.to_lowercase(), new_lobby);
                return warp::http::StatusCode::OK.into_response();
            } else {
                return warp::http::StatusCode::UNAUTHORIZED.into_response();
            }
        },
        None => warp::http::StatusCode::NOT_FOUND.into_response()
    }
}

fn delete_lobby_by_id(
    id: String,
    mgr: Arc<Mutex<LobbyManager>>,
    authorization: Option<String>,
) -> Response {
    let mut manager = mgr.lock().unwrap();
    match manager.lobbies.get(&id.to_lowercase()) {
        Some(lobby) => {
            if lobby.authorize_header(authorization) {
                manager.lobbies.remove(&id);
                return warp::http::StatusCode::OK.into_response();
            } else {
                return warp::http::StatusCode::UNAUTHORIZED.into_response();
            }
        },
        None => warp::http::StatusCode::NOT_FOUND.into_response()
    }
}

fn add_player_to_lobby(
    id: String,
    mgr: Arc<Mutex<LobbyManager>>,
    player: Player,
) -> Response {
    let mut manager = mgr.lock().unwrap();
    match manager.lobbies.get_mut(&id.to_lowercase()) {
        Some(lobby) => {
            lobby.players.insert(player.name.clone(), player);
            return warp::http::StatusCode::OK.into_response();
        },
        None => warp::http::StatusCode::NOT_FOUND.into_response()
    }
}

fn update_player_in_lobby(
    id: String,
    name: String,
    mgr: Arc<Mutex<LobbyManager>>,
    authorization: Option<String>,
    player_update: StrippedPlayer,
) -> Response {
    let mut manager = mgr.lock().unwrap();
    match manager.lobbies.get_mut(&id.to_lowercase()) {
        Some(lobby) => {
            match lobby.players.get(&name) {
                Some(player) => {
                    if player.authorize_header(authorization) {
                        let mut new_player = player.clone();
                        new_player.name = player_update.name;
                        new_player.ready = player_update.ready;
                        lobby.players.remove(&name);
                        lobby.players.insert(new_player.name.clone(), new_player);
                        warp::http::StatusCode::OK.into_response()
                    } else {
                        warp::http::StatusCode::UNAUTHORIZED.into_response()
                    }
                }
                None => warp::http::StatusCode::NOT_FOUND.into_response()
            }
        },
        None => warp::http::StatusCode::NOT_FOUND.into_response()
    }
}

#[tokio::main]
async fn main() {
    let game_server = GameServer::new();
    // TODO: can we run these on the same port? Would that be desirable?
    tokio::spawn(game_server.run_ws_server("127.0.0.1:8080".to_string()));

    let game_manager = Arc::new(Mutex::new(GameManager { game_server }));
    let lobby_manager = Arc::new(Mutex::new(LobbyManager::new(game_manager.clone())));

    let matches_route = warp::path("matches")
        .and(warp::post())
        .and(with_game_manager(game_manager))
        .and(warp::body::json())
        .map(create_match);

    // POST /lobbies
    let post_lobbies_route = warp::path("lobbies")
        .and(warp::path::end())
        .and(warp::post())
        .and(with_lobby_manager(lobby_manager.clone()))
        .and(warp::body::json())
        .map(create_lobby);

    // GET /lobbies
    let get_lobbies_route = warp::path("lobbies")
        .and(warp::path::end())
        .and(warp::get())
        .and(with_lobby_manager(lobby_manager.clone()))
        .map(get_lobbies);

    // GET /lobbies/<id>
    let get_lobbies_id_route = warp::path!("lobbies" / String)
        .and(warp::path::end())
        .and(warp::get())
        .and(with_lobby_manager(lobby_manager.clone()))
        .map(get_lobby_by_id);

    // PUT /lobbies/<id>
    let put_lobbies_id_route = warp::path!("lobbies" / String)
        .and(warp::path::end())
        .and(warp::put())
        .and(with_lobby_manager(lobby_manager.clone()))
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::body::json())
        .map(update_lobby_by_id);

    // DELETE /lobbies/<id>
    let delete_lobbies_id_route = warp::path!("lobbies" / String)
        .and(warp::path::end())
        .and(warp::delete())
        .and(with_lobby_manager(lobby_manager.clone()))
        .and(warp::header::optional::<String>("authorization"))
        .map(delete_lobby_by_id);

    // POST /lobbies/<id>/players
    let post_lobbies_id_players_route = warp::path!("lobbies" / String / "players")
        .and(warp::path::end())
        .and(warp::post())
        .and(with_lobby_manager(lobby_manager.clone()))
        .and(warp::body::json())
        .map(add_player_to_lobby);

    // PUT /lobbies/<id>/players
    let put_lobbies_id_players_route = warp::path!("lobbies" / String / "players" / String)
        .and(warp::path::end())
        .and(warp::put())
        .and(with_lobby_manager(lobby_manager.clone()))
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::body::json())
        .map(update_player_in_lobby);

    let routes = matches_route.or(post_lobbies_route)
                              .or(get_lobbies_id_route)
                              .or(get_lobbies_route)
                              .or(put_lobbies_id_route)
                              .or(delete_lobbies_id_route)
                              .or(post_lobbies_id_players_route)
                              .or(put_lobbies_id_players_route);

    warp::serve(routes).run(([127, 0, 0, 1], 3000)).await;
}
