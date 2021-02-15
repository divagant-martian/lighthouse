use futures::StreamExt;
use std::collections::HashMap;
use std::collections::HashSet;
use task_executor::TaskExecutor;
use tokio_util::time::{delay_queue, DelayQueue};
use types::{
    Attestation, AttesterSlashing, EthSpec, Hash256, ProposerSlashing, SignedAggregateAndProof,
    SignedBeaconBlock, SignedVoluntaryExit, SubnetId,
};

type WorkId = usize;

enum ReprocessWork {
    Att,
    UnAggAtt,
}

#[derive(Default)]
struct ReprocessCache {
    delays: DelayQueue<WorkId>,
    reprocess_works: HashMap<WorkId, (ReprocessWork, delay_queue::Key)>,
    missing_blocks_atts: HashMap<Hash256, HashSet<WorkId>>,
}

pub fn spawn_reprocess_manager(executor: TaskExecutor) {
    let mut cache = ReprocessCache::default();

    let reprocess_future = async move {
        loop {
            tokio::select! {
                work =  cache.delays.next() => {

                }

            }
        }
    };
    executor.spawn(reprocess_future, "FANCY_NAME");
}
