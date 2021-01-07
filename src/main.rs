use std::{collections::HashSet, fs::File, sync::Arc, time::Duration};

use anyhow::{Context as _, Result};
use serenity::{
    async_trait,
    framework::standard::{
        macros::{command, group},
        Args, CommandResult, StandardFramework,
    },
    futures::StreamExt,
    http::Http,
    model::{
        channel::Message,
        gateway::Ready,
        id::{RoleId, UserId},
    },
    prelude::*,
    utils::{content_safe, ContentSafeOptions, MessageBuilder},
};
use tracing::info;
use types::*;

mod types;

const POSITIVE_REACTION: char = '✅';
const NEGATIVE_REACTION: char = '❌';
const SENT_REACTION: char = '📨';
const REACTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

struct ConfigContainer;

impl TypeMapKey for ConfigContainer {
    type Value = Arc<Config>;
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
    }
}

#[group("relay")]
#[commands(forward)]
struct Relay;

#[command("forward")]
async fn forward(ctx: &Context, msg: &Message, mut args: Args) -> CommandResult {
    let config = {
        let data = ctx.data.read().await;
        data.get::<ConfigContainer>().unwrap().clone()
    };

    let delegate_member = if let Ok(member) = ctx
        .http
        .get_member(config.guild_id(), msg.author.id.into())
        .await
    {
        member
    } else {
        msg.channel_id
            .say(ctx, "Umm... have I made your acquaintance?")
            .await?;

        return Ok(());
    };

    if !delegate_member
        .roles
        .contains(&config.delegate_role_id().into())
    {
        msg.channel_id
            .say(ctx, format!("This command is only available to delegates."))
            .await?;

        return Ok(());
    }

    let committee = if let Some(committee) = config
        .committees()
        .iter()
        .find(|&committee| delegate_member.roles.contains(&committee.role_id().into()))
    {
        committee
    } else {
        msg.channel_id
            .say(ctx, "Sorry, but I'm not sure which committee you're on.")
            .await?;

        return Ok(());
    };

    let committee_channel = ctx
        .cache
        .guild_channel(committee.channel_id())
        .await
        .expect("failed to find committee channel");

    let recipient_id = args.single::<UserId>().ok();
    let is_external = recipient_id.is_some();

    let cleaned_content = content_safe(ctx, args.rest(), &ContentSafeOptions::default()).await;

    let typing = msg.channel_id.start_typing(&ctx.http)?;

    let committee_msg = committee_channel
        .say(
            ctx,
            &MessageBuilder::new()
                .push("Received request from ")
                .mention(&msg.author)
                .push(if is_external {
                    format!(
                        " to forward message to {}",
                        &recipient_id.unwrap().mention()
                    )
                } else {
                    String::new()
                })
                .push_line(":")
                .push_quote_line(cleaned_content.clone())
                .push_line("")
                .push(if is_external {
                    "Use the reactions below to approve or deny this request."
                } else {
                    ""
                })
                .push(format!(
                    "Reply to this message{}to send a response.",
                    if is_external { " after voting " } else { " " }
                ))
                .build(),
        )
        .await?;

    if is_external {
        committee_msg.react(ctx, POSITIVE_REACTION).await?;
        committee_msg.react(ctx, NEGATIVE_REACTION).await?;
    }

    msg.reply(
        ctx,
        &MessageBuilder::new()
            .push("Your message has been forwarded to ")
            .push_bold_safe(committee.name())
            .push(if is_external { " for approval" } else { "" })
            .push(".")
            .build(),
    )
    .await?;

    typing.stop();

    if is_external {
        let approved = if let Some(reaction) = committee_msg
            .await_reaction(ctx)
            .timeout(REACTION_TIMEOUT)
            .await
        {
            match reaction
                .as_inner_ref()
                .emoji
                .as_data()
                .chars()
                .next()
                .unwrap()
            {
                POSITIVE_REACTION => {
                    committee_msg
                        .reply(
                            ctx,
                            &MessageBuilder::new()
                                .push("This request has been ")
                                .push_bold("approved")
                                .push(".")
                                .build(),
                        )
                        .await?;

                    true
                }
                NEGATIVE_REACTION => {
                    committee_msg
                        .reply(
                            ctx,
                            &MessageBuilder::new()
                                .push("This request has been ")
                                .push_bold("rejected")
                                .push(".")
                                .build(),
                        )
                        .await?;

                    false
                }
                _ => {
                    committee_msg
                        .reply(ctx, "Invalid reaction; rejecting request.")
                        .await?;

                    false
                }
            }
        } else {
            committee_msg
                .reply(
                    ctx,
                    "No consensus reached in 10 minutes; rejecting request.",
                )
                .await?;

            false
        };

        msg.reply(
            ctx,
            &MessageBuilder::new()
                .push("This request has been ")
                .push_bold(if approved { "approved" } else { "rejected" })
                .push(".")
                .build(),
        )
        .await?;

        if approved {
            recipient_id
                .unwrap()
                .create_dm_channel(ctx)
                .await?
                .say(
                    ctx,
                    &MessageBuilder::new()
                        .push("Received message from ")
                        .mention(&msg.author)
                        .push_line(":")
                        .push_quote_line(cleaned_content.clone()),
                )
                .await?;
        }
    }

    let committee_msg_id = committee_msg.id;

    let mut replies = committee_channel
        .id
        .await_replies(ctx)
        .timeout(REACTION_TIMEOUT)
        .filter(move |msg| match msg.message_reference {
            Some(ref msg_ref) => match msg_ref.message_id {
                Some(m) => m == committee_msg_id,
                None => false,
            },
            None => false,
        })
        .await;

    while let Some(reply_msg) = replies.next().await {
        let cleaned_content = content_safe(
            &ctx.cache,
            &reply_msg.content,
            &ContentSafeOptions::default(),
        )
        .await;

        msg.channel_id
            .say(
                ctx,
                &MessageBuilder::new()
                    .push("Received reply from ")
                    .mention(&reply_msg.author)
                    .push_line(":")
                    .push_quote_line(cleaned_content.clone()),
            )
            .await?;

        reply_msg.react(ctx, SENT_REACTION).await?;
    }

    Ok(())
}

#[group("role")]
#[commands(join)]
struct Role;

#[command("join")]
async fn join(ctx: &Context, msg: &Message, args: Args) -> CommandResult {
    let config = {
        let data = ctx.data.read().await;
        data.get::<ConfigContainer>().unwrap().clone()
    };

    let in_valid_guild = match msg.guild_id {
        Some(id) => id.as_u64() == &config.guild_id(),
        None => false,
    };

    if !in_valid_guild {
        msg.channel_id
            .say(ctx, "I'm not configured to work here.")
            .await?;

        return Ok(());
    }

    let guild = msg.guild(ctx).await.unwrap();

    let query = args.rest().to_lowercase();

    let committee = if let Some(committee) = config.committees().iter().find(|&committee| {
        query == guild.roles[&committee.role_id().into()].name.to_lowercase()
            || query == committee.name()
    }) {
        committee
    } else {
        msg.reply(ctx, "Sorry, I couldn't find a committee by that name.")
            .await?;

        return Ok(());
    };

    let mut member = msg.member(ctx).await?;

    let committee_role_ids: HashSet<RoleId> = config
        .committees()
        .iter()
        .map(|committee| committee.role_id().into())
        .collect();

    let member_role_ids: HashSet<RoleId> = member.roles.iter().copied().collect();

    let other_committee_roles: Vec<_> = committee_role_ids
        .intersection(&member_role_ids)
        .cloned()
        .collect();

    if !other_committee_roles.is_empty() {
        member.remove_roles(ctx, &other_committee_roles).await?;
    }

    let committee_role_id: RoleId = committee.role_id().into();
    let delegate_role_id: RoleId = config.delegate_role_id().into();

    let mut intended_roles = HashSet::with_capacity(2);
    intended_roles.insert(committee_role_id);
    intended_roles.insert(delegate_role_id);

    let roles_to_add: Vec<_> = intended_roles
        .difference(&member_role_ids)
        .cloned()
        .collect();

    if !roles_to_add.is_empty() {
        member.add_roles(ctx, &roles_to_add).await?;
    }

    msg.react(ctx, POSITIVE_REACTION).await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let config_file = File::open("config.ron").context("missing config file")?;
    let config: Config = ron::de::from_reader(config_file).context("invalid config file")?;

    let bot_id = Http::new_with_token(config.token())
        .get_current_application_info()
        .await?
        .id;

    let framework = StandardFramework::new()
        .configure(|c| {
            c.no_dm_prefix(true)
                .with_whitespace(true)
                .on_mention(Some(bot_id))
        })
        .group(&RELAY_GROUP)
        .group(&ROLE_GROUP);

    let mut client = Client::builder(config.token())
        .event_handler(Handler)
        .framework(framework)
        .await
        .context("failed to create client")?;

    {
        let mut data = client.data.write().await;
        data.insert::<ConfigContainer>(Arc::new(config));
    }

    client.start().await.context("failed to start client")?;

    Ok(())
}
