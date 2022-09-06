//! A handler for a guild, each guild will have one handler instance to manage it

use std::{sync::Arc, time::Duration};

use log::{error, info, trace, warn};
use serenity::{
    client::Context,
    model::{
        id::GuildId, prelude::interaction::Interaction,
    }, futures::{StreamExt, stream::FuturesUnordered},
};
use tokio::{
    select,
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    sync::RwLock,
    task::***REMOVED***inHandle,
};

use crate::{AppState, discord_bot::commands::{command, autocomplete, application_command}};
use super::manager::{InternalSender, DiscordEvent};

/// handle an interaction generated by an administrator, e.g. a slash command.
/// matches over the type of interaction and then handles it appropriately, generating a response that can be sent to the user
async fn handle_admin_interaction(interaction: Interaction, context: Context, app_state: AppState) {
    match interaction {
        Interaction::ApplicationCommand(raw_command) => {
            trace!("Received application command: {:?}", raw_command);
            let res = command(&raw_command, &app_state, &context).await;

            match res {
                Ok(response) => {
                    trace!("Sending response: {:?}", response);

                    if let Err(e) = raw_command
                        .create_interaction_response(&context, |f| {
                            *f = response.generate_response();
                            f
                        })
                        .await
                    {
                        error!("Unable to send response: {:?}", e);
                    }
                }
                Err(response) => {
                    response.write_to_log();

                    if let Err(e) = raw_command
                        .create_interaction_response(&context, |f| {
                            *f = response.generate_response();
                            f
                        })
                        .await
                    {
                        error!("Unable to send response: {:?}", e);
                    }
                }
            }
        }
        Interaction::MessageComponent(component) => {
            error!("Received message component: {:?}", component);
        }
        Interaction::Autocomplete(interaction) => {
            let res = autocomplete(&interaction, &app_state, &context).await;
            if let Err(e) = interaction
                .create_autocomplete_response(&context, |f| {
                    match res {
                        Ok(response) => *f = response,
                        Err(response) => response.write_to_log(),
                    }

                    f
                })
                .await
            {
                error!("Unable to send autocomplete response: {:?}", e);
            }
        }
        Interaction::ModalSubmit(submit) => {
            error!("Received modal submit: {:?}", submit);
        }
        // ping commands should not get here
        _ => unreachable!(),
    }
}

/// a handler which manages a guild, interacting with and responding to all events as required
pub struct GuildHandler {
    /// the id of the guild being managed, generated by discord
    pub guild_id: GuildId,
    /// the name of the guild being managed, in plain english
    pub guild_name: String,
    /// the sender to transmit messages to the overall event loop, used to communicate with external modules (e.g. the auth module)
    sender: InternalSender,
    /// access to the general bot context, and data that is stored globally on it, also used for spawning discord tasks to run asynchonrously
    /// inside of the serenity context
    context: Context,
    /// a handle to the database connected to the bot
    app_state: AppState,
    /// the user_id of the bot
    bot_user_id: u64,
    /// a handle to the internal task managing the guild once started
    handle: Option<***REMOVED***inHandle<()>>,
    /// the receiving end of the internal communication channel
    internal_rx: Arc<RwLock<UnboundedReceiver<DiscordEvent>>>,
    /// the sending end of the internal communication channel
    pub internal_tx: UnboundedSender<DiscordEvent>,
}

impl GuildHandler {
    /// create a new handler to manage a guild. A handler will begin by automatically searching for it's message to monitor for interaction.
    /// If the message does not exist, it will automatically create it.
    /// In the future, it will also listen for/manage the state of each guild, e.g. what roles should be assigned.
    /// This should make it very configurable utilising slash commands etc.
    pub fn new(
        guild_id: GuildId,
        guild_name: String,
        context: Context,
        app_state: AppState,
        bot_user_id: u64,
        sender: InternalSender,
    ) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        GuildHandler {
            guild_id,
            guild_name,
            app_state,
            context,
            sender,
            handle: None,
            bot_user_id,
            internal_rx: Arc::new(RwLock::new(rx)),
            internal_tx: tx,
        }
    }

    /// close this handler, at first with a soft close but will force-kill after the provided timeout.
    pub async fn close(&mut self, timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(s) = self.handle.as_mut() {
            tokio::pin!(s);
            self.internal_tx.send(DiscordEvent::Shutdown)?;
            if tokio::time::timeout(timeout, &mut s).await.is_err() {
                warn!(
                    "failed to close handler for {} within timeout period, aborting",
                    self.guild_id
                );
                s.abort();
            }
        }
        self.handle = None;
        Ok(())
    }

    /// begin monitoring a guild for interaction.
    /// note that it is important to not have multiple handlers for the ***REMOVED***e guild.
    pub fn start(&mut self) {
        if self.handle.is_none() {
            let guild = self.guild_id;
            let _sender = self.sender.clone();
            let internal_rx = self.internal_rx.clone();
            let context = self.context.clone();
            let _bot_user_id = self.bot_user_id;
            let app_state = self.app_state.clone();

            info!("Monitoring guild with id {:?}", guild);

            self.handle = Some(tokio::task::spawn(async move {

                // register all commands
                while let Err(e) = guild
                    .set_application_commands(&context, |commands| {
                        *commands = application_command();
                        commands
                    })
                    .await
                {
                    error!("failed to register commands for guild {}: {}", guild, e);
                    tokio::time::sleep(Duration::from_secs(10)).await;
                };

                let mut internal_rx = internal_rx.write().await;
                let mut task_handles = FuturesUnordered::new();

                loop {
                    select! {
                        Some(message) = internal_rx.recv() => {
                            match message {
                                DiscordEvent::Shutdown => {
                                    internal_rx.close();
                                    break;
                                },
                                DiscordEvent::Interaction(interaction) => {
                                    // ignore ping interactions
                                    if matches!(*interaction, Interaction::Ping(_)) {
                                        trace!("ignoring ping interaction");
                                        continue;
                                    }

                                    let t_ctx = context.clone();
                                    let t_app_state = app_state.clone();
                                    task_handles.push(tokio::task::spawn(async move {
                                        handle_admin_interaction(*interaction, t_ctx, t_app_state).await;
                                    }))
                                },
                                e => {
                                    error!("bot ignoring unexpected event: {:?}", e);
                                }
                            }
                        },
                        // drain task handles as they complete
                        _ = task_handles.next(), if !task_handles.is_empty() => {},
                        else => break,
                    }
                }

                // complete all task_handles with a timeout
                if !task_handles.is_empty() {
                    //XXX: timeout is not implemented yet
                    while task_handles.next().await.is_some() {}
                }

                println!("No longer monitoring server with id {:?}", guild);
            }))
        } else {
            eprintln!("Already monitoring guild");
        }
    }
}

impl std::fmt::Debug for GuildHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GuildHandler")
            .field("guild_id", &self.guild_id)
            .field("context", &"no debug information")
            .field("bot_user_id", &self.bot_user_id)
            .field("sender", &"no debug information")
            .field("handle", &self.handle)
            .finish()
    }
}
