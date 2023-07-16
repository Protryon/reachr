#!/bin/bash
set -ex
here=$(realpath $(dirname "$0"))
cd "$here"

if [ -z ${1+x} ] ; then
    echo "missing tag"
    exit 1
fi

export TAG=$1

docker build -t protryon/reachr:$TAG -f ./Dockerfile .
docker push protryon/reachr:$TAG
docker image rm protryon/reachr:$TAG

echo "Uploaded image protryon/reachr:$TAG"
