#!/bin/bash
wrk -d 3 -t 32 -c 100 -s benchmark.lua http://127.0.0.1:8000
