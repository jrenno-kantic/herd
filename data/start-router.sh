#!/bin/bash

scripts/llama-server \
     --models-preset ~/models/32gb/models.ini \
     --models-max 2 \
     --sleep-idle-seconds 300
