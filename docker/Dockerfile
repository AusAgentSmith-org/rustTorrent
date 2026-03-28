# Depends on either outputs of Dockerfile.xx or the binaries compiled elsewhere.
FROM scratch

ADD https://curl.se/ca/cacert.pem /etc/ssl/cacerts.pem

ARG TARGETPLATFORM

## Assumes you are running this from respective context folder.
COPY --chmod=755 ${TARGETPLATFORM}/rtbit /bin/rtbit

WORKDIR /home/rtbit

ENV HOME=/home/rtbit
ENV XDG_DATA_HOME=/home/rtbit/db
ENV XDG_CACHE_HOME=/home/rtbit/cache
ENV SSL_CERT_FILE=/etc/ssl/cacerts.pem

ENV RTBIT_HTTP_API_LISTEN_ADDR=0.0.0.0:3030
ENV RTBIT_LISTEN_PORT=4240

VOLUME /home/rtbit/db
VOLUME /home/rtbit/cache
VOLUME /home/rtbit/downloads
VOLUME /home/rtbit/completed

EXPOSE 3030
EXPOSE 4240
CMD ["server", "start", "/home/rtbit/downloads"]
ENTRYPOINT ["/bin/rtbit"]
