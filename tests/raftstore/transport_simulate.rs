// Copyright 2016 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

use kvproto::raft_serverpb::RaftMessage;
use tikv::raftstore::Result;
use tikv::raftstore::store::Transport;
use rand;
use std::sync::{Arc, RwLock};

use super::util::*;
use self::Strategy::*;

#[derive(Clone)]
pub enum Strategy {
    DropPacket(u32),
    Delay(u64),
    OutOfOrder,
}

trait Filter: Send + Sync {
    // in a SimulateTransport, if any filter's before return true, msg will be discard
    fn before(&self, msg: &RaftMessage) -> bool;
    // with after provided, one can change the return value arbitrarily
    fn after(&self, Result<()>) -> Result<()>;
}

struct FilterDropPacket {
    rate: u32,
}

struct FilterDelay {
    duration: u64,
}

struct FilterOutOfOrder;

impl Filter for FilterDropPacket {
    fn before(&self, _: &RaftMessage) -> bool {
        rand::random::<u32>() % 100u32 < self.rate
    }
    fn after(&self, x: Result<()>) -> Result<()> {
        x
    }
}

impl Filter for FilterDelay {
    fn before(&self, _: &RaftMessage) -> bool {
        sleep_ms(self.duration);
        false
    }
    fn after(&self, x: Result<()>) -> Result<()> {
        x
    }
}

impl Filter for FilterOutOfOrder {
    fn before(&self, _: &RaftMessage) -> bool {
        unimplemented!()
    }
    fn after(&self, _: Result<()>) -> Result<()> {
        unimplemented!()
    }
}

pub struct SimulateTransport<T: Transport> {
    filters: Vec<Box<Filter>>,
    trans: Arc<RwLock<T>>,
}

impl<T: Transport> SimulateTransport<T> {
    pub fn new(strategy: Vec<Strategy>, trans: Arc<RwLock<T>>) -> SimulateTransport<T> {
        let mut filters: Vec<Box<Filter>> = vec![];
        for s in strategy {
            match s {
                DropPacket(rate) => {
                    filters.push(box FilterDropPacket { rate: rate });
                }
                Delay(latency) => {
                    filters.push(box FilterDelay { duration: latency });
                }
                OutOfOrder => {
                    filters.push(box FilterOutOfOrder);
                }
            }
        }

        SimulateTransport {
            filters: filters,
            trans: trans,
        }
    }
}

impl<T: Transport> Transport for SimulateTransport<T> {
    fn send(&self, msg: RaftMessage) -> Result<()> {
        let mut discard = false;
        for strategy in &self.filters {
            if strategy.before(&msg) {
                discard = true;
            }
        }

        let mut res = Ok(());
        if !discard {
            res = self.trans.read().unwrap().send(msg);
        }

        for strategy in self.filters.iter().rev() {
            res = strategy.after(res);
        }

        res
    }
}