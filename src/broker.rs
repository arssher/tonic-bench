use std::pin::Pin;
use std::sync::{Arc, Mutex};

use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc::{self, Sender};
use tonic::{transport::Server, Request, Response, Status};

use neon_broker::neon_broker_server::{NeonBroker, NeonBrokerServer};
use neon_broker::{Empty, SafekeeperTimelineInfo, SubscribeSafekeeperInfoRequest};

pub mod neon_broker {
    tonic::include_proto!("neon_broker"); // The string specified here must match the proto package name
}

struct SharedState {
    senders: Vec<Sender<SafekeeperTimelineInfo>>,
}

struct NeonBrokerService {
    shared_state: Arc<Mutex<SharedState>>,
}

#[tonic::async_trait]
impl NeonBroker for NeonBrokerService {
    // type SubscribeSafekeeperInfoStream = ReceiverStream<Result<SafekeeperTimelineInfo, Status>>;
    type SubscribeSafekeeperInfoStream =
        Pin<Box<dyn Stream<Item = Result<SafekeeperTimelineInfo, Status>> + Send + 'static>>;

    async fn subscribe_safekeeper_info(
        &self,
        _request: Request<SubscribeSafekeeperInfoRequest>,
    ) -> Result<Response<Self::SubscribeSafekeeperInfoStream>, Status> {
        let (tx, mut rx) = mpsc::channel(1000);
        // let tx_clone = tx.clone();
        self.shared_state.lock().unwrap().senders.push(tx);

        // transform rx into stream with item = Result, as method result demands
        let output = async_stream::try_stream! {
            while let Some(info) = rx.recv().await {
                    yield info
                }
        };

        // internal publisher
        // tokio::spawn(async move {
        //     let mut count: u64 = 0;
        //     loop {
        //         let info = SafekeeperTimelineInfo {
        //             last_log_term: 0,
        //             flush_lsn: count,
        //             commit_lsn: 2,
        //             backup_lsn: 3,
        //             remote_consistent_lsn: 4,
        //             peer_horizon_lsn: 5,
        //             safekeeper_connstr: "zenith-1-sk-1.local:7676".to_owned(),
        //         };
        //         count += 1;
        //         if let Err(_) = tx_clone.send(info).await {
        //             return;
        //         }
        //     }
        // });

        Ok(Response::new(
            Box::pin(output) as Self::SubscribeSafekeeperInfoStream
        ))
    }

    async fn publish_safekeeper_info(
        &self,
        request: Request<tonic::Streaming<SafekeeperTimelineInfo>>,
    ) -> Result<Response<Empty>, Status> {
        let subs = self.shared_state.lock().unwrap().senders.clone();

        let mut stream = request.into_inner();
        while let Some(info) = stream.next().await {
            let info = info?;
            for sub in &subs {
                sub.send(info.clone()).await.ok();
            }
        }

        Ok(Response::new(Empty {}))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let neon_broker_service = NeonBrokerService {
        shared_state: Arc::new(Mutex::new(SharedState { senders: vec![] })),
    };

    Server::builder()
        .add_service(NeonBrokerServer::new(neon_broker_service))
        .serve(addr)
        .await?;

    Ok(())
}
