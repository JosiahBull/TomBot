use log::error;
use serenity::{
    async_trait,
    builder::{CreateApplicationCommand, CreateEmbed, CreateInteractionResponse},
    model::prelude::{
        command::CommandOptionType,
        component::ButtonStyle,
        interaction::{
            application_command::{ApplicationCommandInteraction, CommandDataOptionValue},
            message_component::MessageComponentInteraction,
            InteractionResponseType,
        },
        Attachment, AttachmentType,
    },
    prelude::Context,
};

use crate::state::{AppState, FLATMATE_NAMES};

use super::{
    command::{Command, InteractionCommand},
    util::CommandResponse,
};

pub struct PayCommand {}

impl<'a> TryFrom<&'a ApplicationCommandInteraction> for PayCommand {
    type Error = String;

    fn try_from(_: &'a ApplicationCommandInteraction) -> Result<Self, Self::Error> {
        Ok(Self {})
    }
}

#[async_trait]
impl<'a> Command<'a> for PayCommand {
    fn name() -> &'static str {
        "pay"
    }

    fn description() -> &'static str {
        "Create a shared bill for the flat"
    }

    fn get_application_command_options(cmd: &mut CreateApplicationCommand) {
        cmd.create_option(|o| {
            o.name("purpose")
                .description("What is this bill for?")
                .kind(CommandOptionType::String)
                .required(true)
        })
        .create_option(|o| {
            o.name("receipt")
                .description("Attach a photograph of the receipt (if available)")
                .kind(CommandOptionType::Attachment)
                .required(true)
        });

        for flatmate in FLATMATE_NAMES {
            cmd.create_option(|o| {
                o.name(flatmate)
                    .description(format!("The amount for {} to pay.", flatmate))
                    .kind(CommandOptionType::Integer)
                    .required(true)
            });
        }
    }

    async fn handle_application_command<'b>(
        self,
        interaction: &'b ApplicationCommandInteraction,
        state: &'b AppState,
        ctx: &'b Context,
    ) -> Result<CommandResponse<'b>, CommandResponse<'b>> {
        // extract the options
        let options = &interaction.data.options;

        let mut purpose: Option<&str> = None;
        let mut receipt: Option<&Attachment> = None;
        let mut amount: Vec<(&str, i64)> = Vec::with_capacity(FLATMATE_NAMES.len());

        for option in options.iter() {
            match option.name.as_str() {
                "purpose" => {
                    purpose = Some(option.value.as_ref().unwrap().as_str().unwrap());
                }
                "receipt"
                    if matches!(option.resolved, Some(CommandDataOptionValue::Attachment(_))) =>
                {
                    if let Some(CommandDataOptionValue::Attachment(attachment)) = &option.resolved {
                        receipt = Some(attachment);
                    }
                }
                "receipt" => {
                    return Err(CommandResponse::BasicFailure(
                        "Failed to parse receipt as an attachment".to_string(),
                    ));
                }
                _ => {
                    let name = option.name.as_str();
                    let value = option.value.as_ref().unwrap().as_i64().unwrap();
                    amount.push((name, value));
                }
            }
        }

        // if any names aren't present, add them with a value of 0
        for name in FLATMATE_NAMES.iter() {
            if !amount.iter().any(|(n, _)| n == name) {
                amount.push((*name, 0));
            }
        }

        // check if initialisation was successful
        if purpose.is_none() || receipt.is_none() || amount.is_empty() {
            return Err(CommandResponse::BasicFailure(
                "Failed to initialize command".to_string(),
            ));
        }

        let purpose = purpose.unwrap();
        let receipt = receipt.unwrap();

        // try to parse the receipt as a valid reqwest url
        let receipt_url = match reqwest::Url::parse(&receipt.url) {
            Ok(url) => url,
            Err(_) => {
                return Err(CommandResponse::BasicFailure(
                    "Failed to parse receipt as a valid url".to_string(),
                ));
            }
        };

        if let Err(e) = interaction
            .create_interaction_response(&ctx, |f| {
                f.kind(InteractionResponseType::ChannelMessageWithSource)
                    .interaction_response_data(|f| {
                        f.embed(|e| {
                            e.title("Bill created")
                                .description(format!("Bill for {} created", purpose))
                                .color(0xFF0000);

                            for (name, value) in amount.iter() {
                                e.field(
                                    format!(
                                        "Amount for {}{} to pay:",
                                        name[0..1].to_uppercase(),
                                        &name[1..]
                                    ),
                                    value,
                                    false,
                                );
                            }
                            e
                        })
                        .add_file(AttachmentType::Image(receipt_url))
                        .custom_id("pay")
                        .components(|f| {
                            f.create_action_row(|f| {
                                f.create_button(|f| {
                                    f.label("Paid!")
                                        .style(ButtonStyle::Success)
                                        .custom_id("paid")
                                })
                                .create_button(|f| {
                                    f.label("Receipt")
                                        .style(ButtonStyle::Link)
                                        .url(&receipt.url)
                                })
                            })
                        })
                    })
            })
            .await
        {
            return Err(CommandResponse::BasicFailure(format!(
                "Failed to create interaction response: {}",
                e
            )));
        }

        Ok(CommandResponse::NoResponse)
    }
}

#[async_trait]
impl<'a> InteractionCommand<'a> for PayCommand {
    fn answerable<'b>(
        interaction: &'b MessageComponentInteraction,
        app_state: &'b AppState,
        context: &'b Context,
    ) -> bool {
        true //TODO
    }

    async fn interaction<'b>(
        interaction: &'b MessageComponentInteraction,
        state: &'b AppState,
        ctx: &'b Context,
    ) -> Result<CommandResponse<'b>, CommandResponse<'b>> {
        if interaction.member.is_none() {
            return Err(CommandResponse::BasicFailure(
                "Failed to get member".to_string(),
            ));
        }

        let user = &interaction.user;
        let message = &interaction.message;

        if let Err(e) = interaction
            .edit_original_interaction_response(&ctx, |f| {
                // find username of user, edit the message so their name is in bold
                let mut edited_embed = CreateEmbed::default();
                for field in message.embeds[0].fields.iter() {
                    edited_embed.field(field.name.clone(), field.value.clone(), field.inline);
                }

                f.set_embed(edited_embed)
            })
            .await
        {
            return Err(CommandResponse::BasicFailure(format!(
                "Failed to edit interaction response: {}",
                e
            )));
        }

        Ok(CommandResponse::ComplexSuccess(
            CreateInteractionResponse::default()
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|f| {
                    f.content(format!("{} paid!", user.name)).ephemeral(true)
                })
                .to_owned(),
        ))
    }
}
