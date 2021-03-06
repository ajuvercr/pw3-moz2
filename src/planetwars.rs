use futures::stream::futures_unordered::FuturesUnordered;
use futures::{FutureExt, StreamExt};
use mozaic_core::match_context::{MatchCtx, RequestResult};
use tokio::time::Duration;

use serde_json;

use std::convert::TryInto;

pub use planetwars_rules::config::{Config, Map};

use planetwars_rules::protocol as proto;
use planetwars_rules::serializer as pw_serializer;
use planetwars_rules::{PlanetWars, PwConfig};

pub struct PwMatch {
    match_ctx: MatchCtx,
    match_state: PlanetWars,
}

impl PwMatch {
    pub fn create(match_ctx: MatchCtx, config: PwConfig) -> Self {
        // TODO: this is kind of hacked together at the moment
        let match_state = PlanetWars::create(config, match_ctx.players().len());

        PwMatch {
            match_state,
            match_ctx,
        }
    }

    pub async fn run(mut self) {
        while !self.match_state.is_finished() {
            let player_messages = self.prompt_players().await;

            for (player_id, turn) in player_messages {
                self.execute_action(player_id, turn);
            }
            self.match_state.step();

            // Log state
            let state = self.match_state.serialize_state();
            self.match_ctx.emit(serde_json::to_string(&state).unwrap());
        }
    }

    async fn prompt_players(&mut self) -> Vec<(usize, RequestResult<Vec<u8>>)> {
        // borrow these outside closure to make the borrow checker happy
        let state = self.match_state.state();
        let match_ctx = &mut self.match_ctx;

        // TODO: this numbering is really messy.
        // Get rid of the distinction between player_num
        // and player_id.

        self.match_state.state()
            .players
            .iter()
            .filter(|p| p.alive)
            .map(move |player| {
                let state_for_player =
                    pw_serializer::serialize_rotated(&state, player.id-1);
                match_ctx
                    .request(
                        player.id.try_into().unwrap(),
                        serde_json::to_vec(&state_for_player).unwrap(),
                        Duration::from_millis(1000),
                    )
                    .map(move |resp| (player.id, resp))
            })
            .collect::<FuturesUnordered<_>>()
            .collect::<Vec<_>>()
            .await
    }

    fn execute_action(
        &mut self,
        player_num: usize,
        turn: RequestResult<Vec<u8>>,
    ) -> proto::PlayerAction {
        let turn = match turn {
            Err(_timeout) => return proto::PlayerAction::Timeout,
            Ok(data) => data,
        };

        let action: proto::Action = match serde_json::from_slice(&turn) {
            Err(err) => {
                return proto::PlayerAction::ParseError(err.to_string())
            }
            Ok(action) => action,
        };

        let commands = action
            .commands
            .into_iter()
            .map(|command| {
                let res = self.match_state
                    .execute_command(player_num, &command);
                proto::PlayerCommand {
                    command,
                    error: res.err(),
                }
            })
            .collect();

        return proto::PlayerAction::Commands(commands);
    }

}
