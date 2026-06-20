#!/bin/sh
# desc: Deploy a service at a given replica count and report the result
# params: SERVICE, REPLICAS:number
# output: { "ok": bool, "url": string }
# exits: 0 ok "deployed", 1 conflict "a deploy is already in progress", 2 timeout "deploy timed out"
echo "deploying $SERVICE x$REPLICAS"
printf '{"ok":true,"url":"https://%s.svc.example"}' "$SERVICE" > "$SP_OUTPUT_PATH"
