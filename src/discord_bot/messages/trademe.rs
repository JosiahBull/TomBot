use std::time::Duration;

use log::{error, trace};
use serenity::{
    async_trait,
    model::prelude::{ChannelType, Message},
};

use crate::discord_bot::common::distance::load_maps_data_to_embed;

use super::MessageReactor;

const TRADME_LINK_STATE: &str = "https://www.trademe.co.nz/a/property/residential/";

pub struct TrademeDistance;

impl TryFrom<&Message> for TrademeDistance {
    type Error = String;
    fn try_from(_: &Message) -> Result<Self, Self::Error> {
        Ok(TrademeDistance)
    }
}

#[async_trait]
impl<'a> MessageReactor<'a> for TrademeDistance {
    fn name() -> &'static str {
        "trademe-distance"
    }

    fn description() -> &'static str {
        "A simple filter to react and create new threads whenever a trademe property link is sent"
    }

    fn precheck(message: &Message) -> bool {
        message.content.starts_with(TRADME_LINK_STATE)
    }

    async fn process(
        self,
        message: &Message,
        app_state: &crate::state::AppState,
        ctx: &serenity::prelude::Context,
    ) {
        let content = &message.content;
        if content.contains(TRADME_LINK_STATE) {
            let channel_id = message.channel_id;

            // try to parse out the full link if possible
            let links = message
                .content
                .split(' ')
                .find(|p| p.contains(TRADME_LINK_STATE));

            if links.is_none() {
                trace!("stopped trying to parse trademe link - as we were unable to find it");
                return;
            }
            let link = links.unwrap().trim();

            let (tx, rx) = tokio::sync::oneshot::channel();

            app_state
                .trademe_api()
                .add_to_queue(link.to_string(), tx)
                .await;

            // wait for api response, with timeout of 60 minutes
            let response = match tokio::time::timeout(Duration::from_secs(60 * 60), rx).await {
                Ok(r) => r,
                Err(_) => {
                    error!("timed out waiting for trademe api response");
                    return;
                }
            };

            let response = match response {
                Ok(r) => r,
                Err(e) => {
                    error!("failed to get response from trademe api {:?}", e);
                    return;
                }
            };

            let trademe_data = match response {
                Ok(r) => r,
                Err(e) => {
                    error!(
                        "failed to get response from trademe api due to error {:?}",
                        e
                    );
                    return;
                }
            };

            let embed = match load_maps_data_to_embed(trademe_data.address.clone(), app_state).await
            {
                Ok(d) => d,
                Err(e) => {
                    error!("could not create reaction embed for distance: {:?}", e);
                    return;
                }
            };

            let new_channel = match channel_id
                .create_public_thread(ctx, message.id, |f| {
                    f.kind(ChannelType::PublicThread).name(format!(
                        "${}pw - {}",
                        &trademe_data.price, &trademe_data.address
                    ))
                })
                .await
            {
                Ok(c) => c,
                Err(e) => {
                    error!("failed to create new thread in response to trademe message due to error {:?}", e);
                    return;
                }
            };

            let msg = new_channel.send_message(&ctx, |m| m.set_embed(embed)).await;

            if let Err(e) = msg {
                error!(
                    "failed to send trademe distance message to application due to error {:?}",
                    e
                );
            }
        }
    }
}