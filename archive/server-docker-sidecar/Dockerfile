# =============================================================================
# AgentMux HTTP Server
# =============================================================================
FROM node:22-alpine

# Install build dependencies for better-sqlite3
RUN apk add --no-cache python3 make g++ curl

WORKDIR /app

# Copy package files
COPY package.json ./

# Install dependencies
RUN npm install --production=false

# Copy source
COPY tsconfig.json ./
COPY src ./src

# Build TypeScript
RUN npm run build

# Create data directory
RUN mkdir -p /data && chown -R node:node /data

USER node

ENV PORT=3100
ENV HOST=0.0.0.0
ENV DB_PATH=/data/agentmux.db

EXPOSE 3100

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -sf http://localhost:3100/api/health || exit 1

CMD ["node", "dist/index.js"]
