set -euo pipefail
test "$(id -u)" = 1001
docker info >/dev/null
docker compose version
docker buildx version
# The Docker API must not listen on TCP, even within the pod.
if curl --noproxy '*' -fsS --max-time 1 http://127.0.0.1:2375/_ping; then
    exit 1
fi
# Both the initial pod and a resumed pod start with a fresh image store.
if docker image inspect centaur-dind-smoke >/dev/null 2>&1; then
    exit 1
fi
cd /home/agent/workspace
mkdir -p content
printf 'bind-mount-ok' > content/message.txt
printf 'build-ok' > baked.txt
cat > Dockerfile <<'DOCKERFILE'
FROM busybox:1.36.1
COPY baked.txt /www/baked.txt
CMD ["httpd", "-f", "-p", "8080", "-h", "/www"]
DOCKERFILE
cat > compose.yaml <<'COMPOSE'
services:
  web:
    build: .
    image: centaur-dind-smoke
    ports:
      - "127.0.0.1:18080:8080"
    volumes:
      - ./content:/www/shared:ro
      - scratch:/scratch
volumes:
  scratch:
COMPOSE
docker compose up --build -d
for attempt in $(seq 1 30); do
    if curl --noproxy '*' -fsS http://127.0.0.1:18080/baked.txt > response.txt; then
        break
    fi
    sleep 1
done
test "$(cat response.txt)" = build-ok
test "$(curl --noproxy '*' -fsS http://127.0.0.1:18080/shared/message.txt)" = bind-mount-ok
docker compose exec -T web sh -c 'echo volume-ok > /scratch/message.txt'
test "$(docker compose exec -T web cat /scratch/message.txt)" = volume-ok
docker compose down -v
echo DIND_COMPOSE_SMOKE_OK
# Let the test exercise main-container completion and native sidecar shutdown.
read -r _finish
