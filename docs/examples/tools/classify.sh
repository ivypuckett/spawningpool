#!/bin/sh
# desc: Record a deploy-severity classification
# params: LABEL
# output: { "label": string }
echo "recorded: $LABEL"
printf '{"label":"%s"}' "$LABEL" > "$SP_OUTPUT_PATH"
