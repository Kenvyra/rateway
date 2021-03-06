use crate::model::{CacheEntity, CacheRequest};
use futures_util::StreamExt;
use lapin::{
    options::{BasicAckOptions, BasicPublishOptions},
    types::AMQPValue,
    BasicProperties, Channel, Consumer, Error,
};
use log::error;
use serde::Serialize;
use simd_json::{from_slice, to_vec};
use twilight_cache_inmemory::InMemoryCache;
use twilight_gateway::{shard::raw_message::Message, Cluster};

fn serialize_item<T: Serialize>(item: T) -> Option<Vec<u8>> {
    to_vec(&item).ok()
}

pub async fn amqp_reader(
    cluster_id: usize,
    mut consumer: Consumer,
    amqp_channel: Channel,
    cluster: Cluster,
    cache: InMemoryCache,
) -> Result<(), Error> {
    let exchange_name = format!("rateway-{}", cluster_id);
    while let Some(delivery) = consumer.next().await {
        let (channel, mut delivery) = delivery.expect("error in consumer");
        channel
            .basic_ack(delivery.delivery_tag, BasicAckOptions::default())
            .await?;
        match delivery.routing_key.as_str() {
            "cache" => {
                let data = match from_slice::<CacheRequest>(&mut delivery.data) {
                    Ok(d) => d,
                    Err(e) => {
                        error!("Error deserializing cache request: {}", e);
                        continue;
                    }
                };
                // Rust can be annoying
                let send_data = match data.r#type {
                    CacheEntity::CurrentUser => cache.current_user().and_then(serialize_item),
                    CacheEntity::GuildChannel => cache
                        .guild_channel(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::Emoji => cache
                        .emoji(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::Group => cache
                        .group(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::Guild => cache
                        .guild(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::Member => cache
                        .member(data.arguments[0].into(), data.arguments[1].into())
                        .and_then(serialize_item),
                    CacheEntity::Message => cache
                        .message(data.arguments[0].into(), data.arguments[1].into())
                        .and_then(serialize_item),
                    CacheEntity::Presence => cache
                        .presence(data.arguments[0].into(), data.arguments[1].into())
                        .and_then(serialize_item),
                    CacheEntity::PrivateChannel => cache
                        .private_channel(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::Role => cache
                        .role(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::User => cache
                        .user(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::VoiceChannelStates => cache
                        .voice_channel_states(data.arguments[0].into())
                        .and_then(serialize_item),
                    CacheEntity::VoiceState => cache
                        .voice_state(data.arguments[0].into(), data.arguments[1].into())
                        .and_then(serialize_item),
                };
                amqp_channel
                    .basic_publish(
                        &exchange_name,
                        &data.return_routing_key,
                        BasicPublishOptions::default(),
                        send_data.unwrap_or_default(),
                        BasicProperties::default(),
                    )
                    .await?;
            }
            "gateway" => {
                if let Some(headers) = delivery.properties.headers() {
                    if let Some(shard_id) = headers.inner().get("shard_id") {
                        // Sometimes Rust sucks
                        let actual_id = match shard_id {
                            AMQPValue::LongInt(val) => *val as u64,
                            AMQPValue::LongLongInt(val) => *val as u64,
                            AMQPValue::LongUInt(val) => *val as u64,
                            AMQPValue::ShortInt(val) => *val as u64,
                            AMQPValue::ShortShortInt(val) => *val as u64,
                            AMQPValue::ShortShortUInt(val) => *val as u64,
                            AMQPValue::ShortUInt(val) => *val as u64,
                            _ => continue,
                        };
                        if let Err(e) = cluster
                            .send(actual_id, Message::Binary(delivery.data))
                            .await
                        {
                            error!("Error sending gateway command: {}", e);
                        };
                    }
                }
            }
            _ => continue,
        };
    }

    Ok(())
}
