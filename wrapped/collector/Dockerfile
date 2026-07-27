# Build context is the wrapped/ directory (see docker-compose.yml) so the
# generator lib/ ships alongside the collector. node:24 is the current LTS and
# carries node:sqlite without a flag, so the image needs no dependencies to
# install and no native build step.
FROM node:24-slim

ENV NODE_ENV=production \
    PORT=8787 \
    DB_PATH=/data/collector.sqlite \
    WRAPPED_DIR=/data/wrapped

WORKDIR /app
COPY package.json ./
COPY lib ./lib
COPY collector ./collector

# Named volumes inherit this ownership, so the non-root runtime user can write
# the SQLite file and the weekly archive.
RUN mkdir -p /data && chown -R node:node /data /app
USER node

EXPOSE 8787
VOLUME ["/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD node -e "fetch('http://127.0.0.1:'+(process.env.PORT||8787)+'/health').then(r=>process.exit(r.ok?0:1)).catch(()=>process.exit(1))"

CMD ["node", "collector/server.mjs"]
