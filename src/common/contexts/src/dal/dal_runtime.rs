// Copyright 2022 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
use std::io::Error;
use std::io::ErrorKind;
use std::io::Result;
use std::sync::Arc;

use async_trait::async_trait;
use common_base::base::tokio::runtime::Handle;
use opendal::ops::OpCreate;
use opendal::ops::OpDelete;
use opendal::ops::OpList;
use opendal::ops::OpPresign;
use opendal::ops::OpRead;
use opendal::ops::OpStat;
use opendal::ops::OpWrite;
use opendal::ops::PresignedRequest;
use opendal::Accessor;
use opendal::AccessorMetadata;
use opendal::BytesReader;
use opendal::Layer;
use opendal::ObjectMetadata;
use opendal::ObjectStreamer;

/// # TODO
///
/// DalRuntime is used to make sure all IO task are running in the same runtime.
/// So that we will not bothered by `dispatch dropped` panic.
///
/// However, the new processor framework will make sure that all async task running
/// in the same, global, separate, IO only async runtime, so we can remove `DalRuntime`
/// after new processor framework finished.
#[derive(Clone, Debug)]
pub struct DalRuntime {
    inner: Option<Arc<dyn Accessor>>,
    runtime: Handle,
}

impl DalRuntime {
    pub fn new(runtime: Handle) -> Self {
        DalRuntime {
            inner: None,
            runtime,
        }
    }

    fn get_inner(&self) -> Result<Arc<dyn Accessor>> {
        match &self.inner {
            None => Err(Error::new(
                ErrorKind::Other,
                "dal runtime must init wrongly, inner accessor is empty",
            )),
            Some(inner) => Ok(inner.clone()),
        }
    }
}

impl Layer for DalRuntime {
    fn layer(&self, inner: Arc<dyn Accessor>) -> Arc<dyn Accessor> {
        Arc::new(DalRuntime {
            inner: Some(inner),
            runtime: self.runtime.clone(),
        })
    }
}

#[async_trait]
impl Accessor for DalRuntime {
    fn metadata(&self) -> AccessorMetadata {
        self.get_inner()
            .expect("must have valid accessor")
            .metadata()
    }

    async fn create(&self, path: &str, args: OpCreate) -> Result<()> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.create(&path, args).await })
            .await
            .expect("join must success")
    }

    async fn read(&self, path: &str, args: OpRead) -> Result<BytesReader> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.read(&path, args).await })
            .await
            .expect("join must success")
    }

    async fn write(&self, path: &str, args: OpWrite, r: BytesReader) -> Result<u64> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.write(&path, args, r).await })
            .await
            .expect("join must success")
    }

    async fn stat(&self, path: &str, args: OpStat) -> Result<ObjectMetadata> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.stat(&path, args).await })
            .await
            .expect("join must success")
    }

    async fn delete(&self, path: &str, args: OpDelete) -> Result<()> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.delete(&path, args).await })
            .await
            .expect("join must success")
    }

    async fn list(&self, path: &str, args: OpList) -> Result<ObjectStreamer> {
        let op = self.get_inner()?;
        let path = path.to_string();
        self.runtime
            .spawn(async move { op.list(&path, args).await })
            .await
            .expect("join must success")
    }

    fn presign(&self, path: &str, args: OpPresign) -> Result<PresignedRequest> {
        self.get_inner()?.presign(path, args)
    }
}
