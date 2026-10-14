use std::time::Duration;

use centaur_iron_control::IronControlClient;
use centaur_sandbox_agent_k8s::{
    AgentSandboxBackend, AgentSandboxConfig, DindConfig, IronControlSettings,
};
use centaur_sandbox_core::{SandboxBackend, SandboxError, SandboxId, SandboxSpec, SandboxStatus};
use kube::{Client, Config, config::KubeConfigOptions};
use tokio::{
    io::AsyncWriteExt,
    time::{sleep, timeout},
};

#[tokio::test]
#[ignore = "requires a local Kubernetes cluster, sandbox controller, and Compose-capable agent image"]
async fn dind_compose_build_bind_ports_pause_resume_and_shutdown() {
    let context =
        std::env::var("SANDBOX_E2E_K8S_CONTEXT").expect("set an explicit local Kubernetes context");
    let namespace =
        std::env::var("SANDBOX_E2E_K8S_NAMESPACE").expect("set an isolated test namespace");
    let image = std::env::var("SANDBOX_E2E_DIND_AGENT_IMAGE")
        .unwrap_or_else(|_| "centaur-agent:latest".to_owned());
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let client = Client::try_from(
        Config::from_kubeconfig(&KubeConfigOptions {
            context: Some(context),
            ..Default::default()
        })
        .await
        .unwrap(),
    )
    .unwrap();
    let mut config = AgentSandboxConfig::new(
        namespace,
        IronControlSettings {
            client: IronControlClient::new("http://127.0.0.1:1", "test-key"),
            console_url: "http://iron-control".to_owned(),
            control_url: "http://iron-control".to_owned(),
        },
    );
    config.dind = Some(DindConfig::default());
    config.ready_timeout = Duration::from_secs(180);
    let backend = AgentSandboxBackend::new(client, config);
    let handle = backend
        .create(SandboxSpec::new(image).command(["/entrypoint.sh"]).args([
            "/bin/bash",
            "-c",
            include_str!("fixtures/dind-smoke.sh"),
        ]))
        .await
        .expect("create DinD sandbox");
    let result = timeout(Duration::from_secs(240), async {
        wait_for_smoke(&backend, &handle.id).await?;
        let first_instance = backend.observe(&handle.id).await?.instance_id;
        backend.pause(&handle.id).await?;
        // The next pod must run the same startup gate and get a fresh daemon.
        // replicas=0 alone does not prove that Kubernetes has deleted the pod.
        while backend.observe(&handle.id).await?.instance_id.is_some() {
            sleep(Duration::from_millis(500)).await;
        }
        backend.resume(&handle.id).await?;
        if backend.observe(&handle.id).await?.instance_id == first_instance {
            return Err(SandboxError::backend("resume reused the original pod"));
        }
        wait_for_smoke(&backend, &handle.id).await?;
        let mut io = backend.open_io(&handle.id).await?.into_parts();
        io.stdin
            .write_all(b"finish\n")
            .await
            .map_err(|error| SandboxError::io_source("finish smoke", error))?;
        io.stdin
            .flush()
            .await
            .map_err(|error| SandboxError::io_source("flush finish", error))?;
        while backend.status(&handle.id).await? != SandboxStatus::Stopped {
            sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    })
    .await;
    backend
        .stop(&handle.id)
        .await
        .expect("clean up DinD sandbox");
    result
        .expect("DinD Compose smoke timed out")
        .expect("DinD Compose smoke failed");
}

async fn wait_for_smoke(backend: &AgentSandboxBackend, id: &SandboxId) -> Result<(), SandboxError> {
    loop {
        let logs = backend.read_output_since(id, None).await?;
        if logs
            .iter()
            .any(|line| line.contains("DIND_COMPOSE_SMOKE_OK"))
        {
            return Ok(());
        }
        if backend.status(id).await? == SandboxStatus::Stopped {
            return Err(SandboxError::backend(format!(
                "Compose smoke failed: {}",
                logs.join("\n")
            )));
        }
        sleep(Duration::from_millis(500)).await;
    }
}
