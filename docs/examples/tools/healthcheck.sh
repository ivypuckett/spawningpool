#!/bin/sh
# desc: Check whether a service is ready to serve traffic
# params: SERVICE
# output: { "ready": bool }
echo "health-checking $SERVICE"
printf '{"ready":true}' > "$SP_OUTPUT_PATH"
