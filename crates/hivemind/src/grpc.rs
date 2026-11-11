//! gRPC shared-service surface (M5).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use hivemind_engine::{Durability, Options, Store};
use serde_json::{Value, json};
use tonic::{Request, Response, Status};
use tracing::info;

use crate::oauth::OAuthState;
use crate::service::Service;
use crate::sync;
use crate::tenant::{self, ensure_tenant_dir};

pub mod pb {
    tonic::include_proto!("hivemind.v1");
}

use pb::hivemind_server::{Hivemind, HivemindServer};
use pb::{
    ChangesSinceRequest, ChangesSinceResponse, HelloRequest, HelloResponse, RpcRequest,
    RpcResponse,
};

#[derive(Clone)]
struct GrpcState {
    root: PathBuf,
    token: String,
    oauth: OAuthState,
    durability: Durability,
    stores: Arc<Mutex<HashMap<String, Arc<Service>>>>,
}

impl std::fmt::Debug for GrpcState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrpcState")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct GrpcArgs {
    pub root: PathBuf,
    pub listen: SocketAddr,
    pub token: String,
    pub oauth: OAuthState,
    pub no_fsync: bool,
}

pub async fn serve(args: GrpcArgs) -> Result<()> {
    let state = GrpcState {
        root: args.root.clone(),
        token: args.token,
        oauth: args.oauth,
        durability: if args.no_fsync {
            Durability::NoFsync
        } else {
            Durability::Fsync
        },
        stores: Arc::new(Mutex::new(HashMap::new())),
    };
    let svc = HivemindServer::new(state);
    info!(%args.listen, "hivemind gRPC service listening");
    tonic::transport::Server::builder()
        .add_service(svc)
        .serve(args.listen)
        .await
        .with_context(|| format!("grpc serve {}", args.listen))?;
    Ok(())
}

#[tonic::async_trait]
impl Hivemind for GrpcState {
    async fn hello(&self, request: Request<HelloRequest>) -> Result<Response<HelloResponse>, Status> {
        self.authorize(&request)?;
        let tenant = request.into_inner().tenant;
        let service = self.service_for(&tenant)?;
        let json = serde_json::to_string(&service.hello())
            .map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(HelloResponse { json }))
    }

    async fn rpc(&self, request: Request<RpcRequest>) -> Result<Response<RpcResponse>, Status> {
        self.authorize(&request)?;
        let inner = request.into_inner();
        let service = self.service_for(&inner.tenant)?;
        let params: Value = if inner.params_json.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&inner.params_json)
                .map_err(|e| Status::invalid_argument(format!("params_json: {e}")))?
        };
        let value = service
            .dispatch(&inner.method, params)
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let json =
            serde_json::to_string(&value).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(RpcResponse { json }))
    }

    async fn changes_since(
        &self,
        request: Request<ChangesSinceRequest>,
    ) -> Result<Response<ChangesSinceResponse>, Status> {
        self.authorize(&request)?;
        let inner = request.into_inner();
        let service = self.service_for(&inner.tenant)?;
        let limit = if inner.limit == 0 { 100 } else { inner.limit as usize };
        let value = service
            .dispatch(
                "changes_since",
                json!({ "from": inner.from, "limit": limit }),
            )
            .map_err(|e| Status::invalid_argument(e.to_string()))?;
        let json =
            serde_json::to_string(&value).map_err(|e| Status::internal(e.to_string()))?;
        Ok(Response::new(ChangesSinceResponse { json }))
    }
}

impl GrpcState {
    fn authorize<T>(&self, request: &Request<T>) -> Result<(), Status> {
        let header = request
            .metadata()
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        let token = sync::bearer_token(header).map_err(|e| Status::unauthenticated(e.to_string()))?;
        self.oauth
            .validate_bearer(token, &self.token)
            .map_err(|e| Status::unauthenticated(e.to_string()))
    }

    fn service_for(&self, tenant: &str) -> Result<Arc<Service>, Status> {
        tenant::validate_tenant_id(tenant).map_err(|e| Status::invalid_argument(e.to_string()))?;
        {
            let guard = self
                .stores
                .lock()
                .map_err(|_| Status::internal("lock poisoned"))?;
            if let Some(svc) = guard.get(tenant) {
                return Ok(svc.clone());
            }
        }
        let dir = ensure_tenant_dir(&self.root, tenant)
            .map_err(|e| Status::internal(e.to_string()))?;
        let store = Store::open(
            &dir,
            Options {
                durability: self.durability,
                ..Options::default()
            },
        )
        .map_err(|e| Status::internal(e.to_string()))?;
        let service = Arc::new(Service::new(store));
        let mut guard = self
            .stores
            .lock()
            .map_err(|_| Status::internal("lock poisoned"))?;
        Ok(guard.entry(tenant.to_owned()).or_insert(service).clone())
    }
}
