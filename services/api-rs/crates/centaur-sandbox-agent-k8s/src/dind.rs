use centaur_sandbox_core::ResourceRequirements;
use serde_json::{Value, json};

use super::{insert_optional, tools::AGENT_UID, upsert_env};

pub const DEFAULT_DIND_IMAGE: &str = "docker:29.9.0-dind";
const DOCKER_HOST: &str = "unix:///run/centaur-docker/docker.sock";
const SOCKET_DIR: &str = "/run/centaur-docker";

#[derive(Clone, Debug)]
pub struct DindConfig {
    pub image: String,
    pub image_pull_policy: Option<String>,
    pub resources: Option<ResourceRequirements>,
}

impl Default for DindConfig {
    fn default() -> Self {
        Self {
            image: DEFAULT_DIND_IMAGE.to_owned(),
            image_pull_policy: Some("IfNotPresent".to_owned()),
            resources: None,
        }
    }
}

pub(crate) fn sidecar_json(
    config: &DindConfig,
    agent_env: &mut Vec<(String, String)>,
    volumes: &mut Vec<Value>,
    agent_mounts: &mut Vec<Value>,
) -> Value {
    // The daemon resolves bind-mount sources in its own filesystem. Share the
    // workspace and existing mounts at identical paths, including subPaths and
    // read-only repo-cache mounts. Image-only files outside these mounts are
    // deliberately not bind-mountable.
    let workspace = "/home/agent/workspace";
    if !agent_mounts.iter().any(|mount| {
        mount["mountPath"]
            .as_str()
            .is_some_and(|path| std::path::Path::new(workspace).starts_with(path))
    }) {
        volumes.push(json!({"name": "dind-workspace", "emptyDir": {}}));
        agent_mounts.push(json!({"name": "dind-workspace", "mountPath": workspace}));
    }
    volumes.extend([
        json!({"name": "dind-socket", "emptyDir": {}}),
        json!({"name": "dind-data", "emptyDir": {}}),
    ]);
    agent_mounts.push(json!({"name": "dind-socket", "mountPath": SOCKET_DIR}));
    let mut daemon_mounts = agent_mounts.clone();
    daemon_mounts.push(json!({"name": "dind-data", "mountPath": "/var/lib/docker"}));

    // Forward only daemon egress settings, never the agent's credential env.
    let mut daemon_env: Vec<Value> = agent_env
        .iter()
        .filter(|(name, _)| {
            matches!(
                name.as_str(),
                "HTTP_PROXY"
                    | "HTTPS_PROXY"
                    | "NO_PROXY"
                    | "http_proxy"
                    | "https_proxy"
                    | "no_proxy"
                    | "SSL_CERT_FILE"
                    | "SSL_CERT_DIR"
            )
        })
        .map(|(name, value)| json!({"name": name, "value": value}))
        .collect();
    daemon_env.extend([
        json!({"name": "DOCKER_HOST", "value": DOCKER_HOST}),
        json!({"name": "DOCKER_TLS_CERTDIR", "value": ""}),
    ]);
    agent_env.retain(|(name, _)| {
        !matches!(
            name.as_str(),
            "DOCKER_CONTEXT" | "DOCKER_TLS_VERIFY" | "DOCKER_CERT_PATH"
        )
    });
    upsert_env(agent_env, "DOCKER_HOST", DOCKER_HOST.to_owned());
    upsert_env(agent_env, "DOCKER_TLS_CERTDIR", String::new());

    let probe = json!({
        "exec": {"command": ["docker", "info"]},
        "periodSeconds": 1,
        "timeoutSeconds": 2,
    });
    let mut startup_probe = probe.clone();
    startup_probe["failureThreshold"] = json!(60);
    let mut sidecar = json!({
        "name": "dind",
        "image": config.image,
        "restartPolicy": "Always",
        // Passing dockerd explicitly avoids the image entrypoint's default
        // TCP listener. This socket is accessible only inside this sandbox.
        "args": ["dockerd", format!("--host={DOCKER_HOST}"), format!("--group={AGENT_UID}")],
        "env": daemon_env,
        "securityContext": {"privileged": true, "runAsUser": 0, "runAsGroup": 0},
        "volumeMounts": daemon_mounts,
        "startupProbe": startup_probe,
        "readinessProbe": probe,
    });
    insert_optional(
        &mut sidecar,
        "imagePullPolicy",
        config.image_pull_policy.clone(),
    );
    insert_optional(&mut sidecar, "resources", config.resources.as_ref());
    sidecar
}
