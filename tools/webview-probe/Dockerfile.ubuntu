ARG UBUNTU=22.04
FROM ubuntu:${UBUNTU}
RUN apt-get -qq update && DEBIAN_FRONTEND=noninteractive apt-get -qq install -y --no-install-recommends python3-gi gir1.2-webkit2-4.1 xvfb xauth > /dev/null && rm -rf /var/lib/apt/lists/*
