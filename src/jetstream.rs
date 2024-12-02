use anyhow::{Error, Result};
use async_nats::jetstream::stream::RetentionPolicy::WorkQueue;
use async_nats::jetstream::stream::StorageType::Memory;
use async_nats::jetstream::stream::Stream;
use async_nats::jetstream::{self, consumer::PullConsumer, Context};
use async_nats::Client;
use futures::StreamExt;
use prost::Message;
use std::cmp::min;
use std::fmt::Debug;
use std::time::Duration;

#[derive(Default, Clone, Debug)]
pub struct JetStreamConfig {
    /// the url of the nats server
    pub nats_url: String,
    /// the name of the stream
    pub stream_name: String,
    /// the name of the durable consumer
    pub durable_name: String,
    /// subjects of the stream
    pub subjects: Vec<String>,
    /// milliseconds - How long to allow messages to remain un-acknowledged before attempting redeliver
    pub ack_wait: u64,
    /// Maximum number of times a specific message will be delivered. Use this to avoid poison pill messages that repeatedly crash your consumer processes forever
    pub max_deliver: i64,
    /// milliseconds - The initial waiting period after failure before the message can be processed again, on retry the delay will be doubled until max_deliver count is reached
    pub redelivery_delay: u64,
    /// Maximum number of messages to be processed at once in a batch
    pub batch_size: usize,
    /// milliseconds - The maximum time to wait for a batch to be filled before processing if the batch is not full
    pub batch_timeout: u64,
    /// milliseconds - The time to send a heartbeat to the server to keep the connection alive
    pub heartbeat: u64,
    /// max number of messages that the stream can hold before it starts dropping messages
    pub max_messages: i64,
    /// How large the Stream may become in total bytes before the configured discard policy kicks in
    pub max_bytes: i64,
}

#[derive(Clone, Debug)]
pub struct JetStreamService {
    client: Client,
    context: Context,
    config: JetStreamConfig,
}

impl JetStreamService {
    pub async fn new(config: JetStreamConfig) -> Result<Self> {
        let client = match async_nats::connect(config.nats_url.clone()).await {
            Ok(client) => {
                println!("Successfully connected to NATS JetStream server");
                client
            }
            Err(err) => {
                eprintln!("Unable to connect to NATS JetStream server: {:?}", err);
                return Err(Error::from(err));
            }
        };
        let context = jetstream::new(client.clone());

        Ok(Self {
            client,
            context,
            config,
        })
    }

    pub async fn publish<T: Message + Debug + Send + Sync>(
        &self,
        subject: &str,
        message: &T,
    ) -> Result<()> {
        let payload = message.encode_to_vec();
        let _stream = self.get_or_create_stream().await?;
        let _ack = self
            .context
            .publish(subject.to_string(), payload.into())
            .await?;
        Ok(())
    }

    pub async fn subscribe<T, F>(&self, stream: Stream, on_message: F) -> Result<()>
        where
            T: Message + Default + Send + Sync + 'static,
            F: Fn(T) -> Result<(), ()> + Send + Sync + 'static + Clone,
    {
        // Create or get a durable pull-based consumer
        let consumer: PullConsumer = match stream.get_consumer(&self.config.durable_name).await {
            Ok(consumer) => consumer,
            Err(_) => {
                match stream
                    .create_consumer(jetstream::consumer::pull::Config {
                        durable_name: Some(self.config.durable_name.clone()),
                        ack_policy: jetstream::consumer::AckPolicy::Explicit,
                        ack_wait: Duration::from_millis(self.config.ack_wait),
                        max_deliver: self.config.max_deliver,
                        ..Default::default()
                    })
                    .await
                {
                    Ok(consumer) => consumer,
                    Err(e) => {
                        eprintln!("Failed to create consumer: {:?}", e);
                        return Err(Error::from(e));
                    }
                }
            }
        };

        println!(
            "Subscribed with pull consumer, durable name: {}",
            self.config.durable_name
        );

        // Add exponential backoff for failed pulls
        let mut backoff_duration = Duration::from_millis(self.config.redelivery_delay);
        // Fail-safe in case max_deliver is not set correctly
        let max_backoff = Duration::from_millis(self.config.redelivery_delay * 10);

        loop {
            // Use fetch with a timeout to prevent aggressive pulling
            match consumer
                .fetch()
                .max_messages(self.config.batch_size)
                .expires(Duration::from_millis(self.config.batch_timeout))
                .heartbeat(Duration::from_millis(self.config.heartbeat))
                .messages()
                .await
            {
                Ok(mut messages) => {
                    // Reset backoff on successful pull
                    backoff_duration = Duration::from_millis(self.config.redelivery_delay);

                    if let Some(Ok(message)) = messages.next().await {
                        // if let Ok(data) = serde_json::from_slice::<T>(&message.payload) {
                        if let Ok(data) = T::decode(&*message.payload) {
                            match on_message(data) {
                                Ok(_) => {
                                    // Immediately ack the message
                                    if let Err(e) = message.ack().await {
                                        eprintln!("Failed to ack message: {:?}", e);
                                    }
                                }
                                Err(_) => {
                                    println!("Message handling failed, will trigger redelivery");
                                    // No ack, so message will be redelivered
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to fetch messages: {:?}", e);
                    // Implement exponential backoff
                    tokio::time::sleep(backoff_duration).await;
                    backoff_duration = min(backoff_duration * 2, max_backoff);
                }
            }

            // Add a small delay between pulls to prevent overwhelming the server
            tokio::time::sleep(Duration::from_millis(self.config.redelivery_delay)).await;
        }
    }

    pub async fn get_or_create_stream(&self) -> Result<Stream> {
        let stream = match self
            .context
            .get_or_create_stream(jetstream::stream::Config {
                name: self.config.stream_name.to_string(),
                subjects: self.config.subjects.clone(),
                max_messages: self.config.max_messages,
                max_bytes: self.config.max_bytes,
                retention: WorkQueue,
                storage: Memory,
                num_replicas: 1,
                max_consumers: 0,
                ..Default::default()
            })
            .await
        {
            Ok(stream) => {
                println!(
                    "Successfully got or created stream: {}",
                    self.config.stream_name
                );
                stream
            }
            Err(err) => {
                eprintln!("Unable to get or create stream: {:?}", err);
                return Err(Error::from(err));
            }
        };
        Ok(stream)
    }
}
