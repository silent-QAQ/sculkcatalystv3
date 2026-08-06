package com.sculkcatalyst.paperbridge.client;

import com.google.gson.JsonObject;
import com.sculkcatalyst.paperbridge.config.BridgeConfig;
import com.sculkcatalyst.paperbridge.protocol.BridgeEnvelope;
import com.sculkcatalyst.paperbridge.protocol.BridgeMessageType;
import com.sculkcatalyst.paperbridge.protocol.BridgePayloads;
import com.sculkcatalyst.paperbridge.protocol.HmacSigner;
import com.sculkcatalyst.paperbridge.protocol.ProtocolCodec;
import com.sculkcatalyst.paperbridge.protocol.ProtocolException;
import com.sculkcatalyst.paperbridge.protocol.ProtocolFactory;
import java.net.http.HttpClient;
import java.net.http.WebSocket;
import java.nio.ByteBuffer;
import java.time.Duration;
import java.util.ArrayDeque;
import java.util.Base64;
import java.util.List;
import java.util.Objects;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.RejectedExecutionException;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.BiConsumer;
import java.util.function.Consumer;
import java.util.function.LongConsumer;

/**
 * Owns all WebSocket, v2 handshake, signing, replay protection, and reconnect work.
 * It never invokes Bukkit or PlaceholderAPI directly.
 */
public final class BridgeWebSocketClient implements AutoCloseable {
    private static final int MAX_INBOUND_CHARS = 512 * 1024;
    private static final int NONCE_BYTES = 24;
    private static final long MAX_CLOCK_SKEW_MILLIS = 30_000L;
    private static final long MAX_SEQUENCE_GAP = 100_000L;

    private final BridgeConfig config;
    private final String instanceId;
    private final ProtocolCodec codec;
    private final BiConsumer<Long, BridgeEnvelope> inboundConsumer;
    private final Consumer<HandshakeChallenge> challengeConsumer;
    private final LongConsumer readyConsumer;
    private final LongConsumer disconnectedConsumer;
    private final Consumer<String> statusLogger;
    private final ExecutorService ioExecutor;
    private final ScheduledExecutorService reconnectExecutor;
    private final HttpClient httpClient;
    private final ArrayBlockingQueue<OutboundIntent> outbound;
    private final AtomicBoolean started = new AtomicBoolean();
    private final AtomicBoolean stopping = new AtomicBoolean();
    private final AtomicBoolean reconnectScheduled = new AtomicBoolean();
    private final AtomicBoolean outboundDrainScheduled = new AtomicBoolean();
    private final AtomicInteger reconnectAttempts = new AtomicInteger();
    private final AtomicLong epochCounter = new AtomicLong();
    private final AtomicLong activeEpoch = new AtomicLong();
    private final AtomicLong readyEpoch = new AtomicLong();
    private final AtomicLong awaitingHelloEpoch = new AtomicLong();
    private final AtomicReference<WebSocket> activeSocket = new AtomicReference<>();

    // The following fields are owned exclusively by ioExecutor.
    private Connection connection;
    private long pendingEpoch;

    public BridgeWebSocketClient(
        BridgeConfig config,
        String instanceId,
        ProtocolCodec codec,
        BiConsumer<Long, BridgeEnvelope> inboundConsumer,
        Consumer<HandshakeChallenge> challengeConsumer,
        LongConsumer readyConsumer,
        LongConsumer disconnectedConsumer,
        Consumer<String> statusLogger
    ) {
        this.config = Objects.requireNonNull(config, "config");
        this.instanceId = requireNonBlank(instanceId, "instanceId");
        this.codec = Objects.requireNonNull(codec, "codec");
        this.inboundConsumer = Objects.requireNonNull(inboundConsumer, "inboundConsumer");
        this.challengeConsumer = Objects.requireNonNull(challengeConsumer, "challengeConsumer");
        this.readyConsumer = Objects.requireNonNull(readyConsumer, "readyConsumer");
        this.disconnectedConsumer = Objects.requireNonNull(disconnectedConsumer, "disconnectedConsumer");
        this.statusLogger = Objects.requireNonNull(statusLogger, "statusLogger");
        this.ioExecutor = Executors.newSingleThreadExecutor(namedThreadFactory("sculk-bridge-io"));
        this.reconnectExecutor = Executors.newSingleThreadScheduledExecutor(namedThreadFactory("sculk-bridge-reconnect"));
        this.httpClient = HttpClient.newBuilder()
            .connectTimeout(config.network().connectTimeout())
            .executor(ioExecutor)
            .build();
        this.outbound = new ArrayBlockingQueue<>(config.network().outboundQueueCapacity());
    }

    public void start() {
        if (started.compareAndSet(false, true)) {
            scheduleConnect(Duration.ZERO);
        }
    }

    /**
     * Submits the authenticated hello after the plugin has gathered capabilities on the global
     * scheduler. The epoch check prevents an old challenge callback from confirming a new socket.
     */
    public boolean sendHello(long epoch, List<String> capabilities) {
        if (!isOperational()) {
            return false;
        }
        List<String> safeCapabilities = capabilities == null ? List.of() : List.copyOf(capabilities);
        return submitIo(() -> submitHello(epoch, safeCapabilities));
    }

    /** Queues an authenticated business frame for exactly one ready connection epoch. */
    public boolean send(long epoch, BridgeMessageType type, String requestId, Object payload) {
        Objects.requireNonNull(type, "type");
        if (!isReady(epoch)) {
            return false;
        }
        JsonObject payloadObject;
        try {
            payloadObject = codec.payloadOf(payload);
        } catch (RuntimeException exception) {
            statusLogger.accept("Bridge refused outbound " + type.wireName() + ": " + conciseError(exception));
            return false;
        }
        if (!outbound.offer(new OutboundIntent(epoch, type, requestId, payloadObject.deepCopy()))) {
            statusLogger.accept("Bridge outbound queue is full; dropping " + type.wireName());
            return false;
        }
        scheduleOutboundDrain();
        return true;
    }

    public boolean isConnected() {
        return activeSocket.get() != null && !stopping.get();
    }

    public long readyEpoch() {
        return readyEpoch.get();
    }

    public boolean isReady(long epoch) {
        return epoch > 0 && !stopping.get() && readyEpoch.get() == epoch && activeEpoch.get() == epoch;
    }

    public boolean isAwaitingHello(long epoch) {
        return epoch > 0
            && !stopping.get()
            && awaitingHelloEpoch.get() == epoch
            && activeEpoch.get() == epoch;
    }

    @Override
    public void close() {
        if (!stopping.compareAndSet(false, true)) {
            return;
        }
        started.set(false);
        readyEpoch.set(0L);
        awaitingHelloEpoch.set(0L);
        activeEpoch.set(0L);
        outbound.clear();
        reconnectExecutor.shutdownNow();

        WebSocket socket = activeSocket.getAndSet(null);
        if (socket != null) {
            socket.abort();
        }
        submitIo(this::closeFromIo);
        ioExecutor.shutdown();
    }

    private void closeFromIo() {
        if (connection != null) {
            cancelHandshakeTimeout(connection);
            connection.outboundFrames.clear();
            connection = null;
        }
        pendingEpoch = 0L;
    }

    private boolean isOperational() {
        return started.get() && !stopping.get();
    }

    private void scheduleConnect(Duration delay) {
        if (!isOperational() || !reconnectScheduled.compareAndSet(false, true)) {
            return;
        }
        reconnectExecutor.schedule(() -> {
            reconnectScheduled.set(false);
            submitIo(this::beginConnect);
        }, delay.toMillis(), TimeUnit.MILLISECONDS);
    }

    private void beginConnect() {
        if (!isOperational() || connection != null || pendingEpoch != 0L) {
            return;
        }
        long epoch = epochCounter.incrementAndGet();
        pendingEpoch = epoch;
        try {
            httpClient.newWebSocketBuilder()
                .connectTimeout(config.network().connectTimeout())
                .buildAsync(config.backendWsUri(), new Listener(epoch))
                .orTimeout(config.network().connectTimeout().toMillis(), TimeUnit.MILLISECONDS)
                .whenComplete((socket, error) -> submitIo(() -> completeConnectAttempt(epoch, error)));
        } catch (RuntimeException exception) {
            completeConnectAttempt(epoch, exception);
        }
    }

    private void completeConnectAttempt(long epoch, Throwable error) {
        if (error == null || pendingEpoch != epoch) {
            return;
        }
        pendingEpoch = 0L;
        if (isOperational()) {
            statusLogger.accept("Bridge connection failed: " + conciseError(error));
            scheduleReconnect();
        }
    }

    private void activateConnection(long epoch, WebSocket socket) {
        if (!isOperational() || pendingEpoch != epoch || connection != null) {
            socket.abort();
            return;
        }
        pendingEpoch = 0L;
        Connection next = new Connection(epoch, socket, new ProtocolFactory(config.serverId(), instanceId, codec));
        next.clientNonce = createNonce();
        connection = next;
        activeSocket.set(socket);
        activeEpoch.set(epoch);
        readyEpoch.set(0L);
        awaitingHelloEpoch.set(0L);
        outbound.clear();
        scheduleHandshakeTimeout(next);

        enqueueFrame(
            next,
            next.factory.createUnsigned(
                BridgeMessageType.HELLO_INIT,
                null,
                null,
                new BridgePayloads.HelloInit(next.clientNonce)
            )
        );
    }

    private void scheduleHandshakeTimeout(Connection current) {
        current.handshakeTimeout = reconnectExecutor.schedule(
            () -> submitIo(() -> {
                if (connection == current && current.phase != ConnectionPhase.READY) {
                    terminateConnection(current, "handshake timeout", true, true);
                }
            }),
            config.network().connectTimeout().toMillis(),
            TimeUnit.MILLISECONDS
        );
    }

    private void submitHello(long epoch, List<String> capabilities) {
        Connection current = connection;
        if (current == null || current.epoch != epoch || current.phase != ConnectionPhase.WAITING_HELLO) {
            return;
        }
        if (current.challengeExpiresAt <= System.currentTimeMillis()) {
            terminateConnection(current, "challenge expired before hello", true, true);
            return;
        }
        BridgeEnvelope hello = current.factory.createSigned(
            BridgeMessageType.HELLO,
            null,
            null,
            new BridgePayloads.Hello(current.clientNonce, current.serverNonce, capabilities, null),
            config.token(),
            "hello"
        );
        current.phase = ConnectionPhase.WAITING_ACK;
        awaitingHelloEpoch.compareAndSet(epoch, 0L);
        enqueueFrame(current, hello);
    }

    private void scheduleOutboundDrain() {
        if (outboundDrainScheduled.compareAndSet(false, true) && !submitIo(this::drainOutbound)) {
            outboundDrainScheduled.set(false);
        }
    }

    private void drainOutbound() {
        outboundDrainScheduled.set(false);
        Connection current = connection;
        if (current == null || current.phase != ConnectionPhase.READY) {
            outbound.clear();
            return;
        }
        while (current.outboundFrames.size() < config.network().outboundQueueCapacity()) {
            OutboundIntent intent = outbound.poll();
            if (intent == null) {
                break;
            }
            if (intent.epoch != current.epoch || current.phase != ConnectionPhase.READY) {
                continue;
            }
            enqueueFrame(
                current,
                current.factory.createSigned(
                    intent.type,
                    intent.requestId,
                    current.sessionId,
                    intent.payload,
                    current.c2sKey,
                    "c2s"
                )
            );
        }
        if (!outbound.isEmpty() && current.phase == ConnectionPhase.READY) {
            scheduleOutboundDrain();
        }
    }

    private void enqueueFrame(Connection current, BridgeEnvelope envelope) {
        if (connection != current) {
            return;
        }
        current.outboundFrames.addLast(codec.encode(envelope));
        flushNext(current);
    }

    private void flushNext(Connection current) {
        if (connection != current || current.sendInFlight || current.outboundFrames.isEmpty()) {
            return;
        }
        String message = current.outboundFrames.removeFirst();
        current.sendInFlight = true;
        try {
            current.socket.sendText(message, true)
                .orTimeout(config.network().connectTimeout().toMillis(), TimeUnit.MILLISECONDS)
                .whenComplete((ignored, error) -> submitIo(() -> completeWrite(current, error)));
        } catch (RuntimeException exception) {
            completeWrite(current, exception);
        }
    }

    private void completeWrite(Connection current, Throwable error) {
        if (connection != current) {
            return;
        }
        current.sendInFlight = false;
        if (error != null) {
            statusLogger.accept("Bridge write failed: " + conciseError(error));
            terminateConnection(current, "write failure", true, true);
            return;
        }
        flushNext(current);
        if (!outbound.isEmpty()) {
            scheduleOutboundDrain();
        }
    }

    private void handleMessage(long epoch, WebSocket socket, String rawMessage) {
        Connection current = connection;
        if (!isOperational() || current == null || current.epoch != epoch || current.socket != socket) {
            return;
        }
        BridgeEnvelope envelope;
        try {
            envelope = codec.decode(rawMessage);
        } catch (ProtocolException exception) {
            reject(current, "invalid envelope: " + exception.getMessage());
            return;
        }
        if (!config.serverId().equals(envelope.serverId()) || !instanceId.equals(envelope.instanceId())) {
            reject(current, "mismatched server_id or instance_id");
            return;
        }
        switch (current.phase) {
            case WAITING_CHALLENGE -> handleChallenge(current, envelope);
            case WAITING_HELLO -> reject(current, "received a frame before hello was submitted");
            case WAITING_ACK -> handleHelloAck(current, envelope);
            case READY -> handleAuthenticated(current, envelope);
        }
    }

    private void handleChallenge(Connection current, BridgeEnvelope envelope) {
        if (envelope.type() != BridgeMessageType.CHALLENGE || envelope.sessionId() != null || envelope.signature() != null) {
            reject(current, "expected an unsigned challenge with a null session_id");
            return;
        }
        BridgePayloads.Challenge challenge;
        try {
            challenge = codec.parsePayload(envelope, BridgePayloads.Challenge.class);
        } catch (ProtocolException exception) {
            reject(current, "invalid challenge payload");
            return;
        }
        if (!current.clientNonce.equals(challenge.clientNonce())
            || !validNonce(challenge.serverNonce())
            || challenge.expiresAt() <= System.currentTimeMillis()) {
            reject(current, "challenge nonce or expiration did not match this connection");
            return;
        }
        current.serverNonce = challenge.serverNonce();
        current.challengeExpiresAt = challenge.expiresAt();
        current.phase = ConnectionPhase.WAITING_HELLO;
        awaitingHelloEpoch.set(current.epoch);
        try {
            challengeConsumer.accept(new HandshakeChallenge(current.epoch, current.clientNonce, current.serverNonce));
        } catch (RuntimeException exception) {
            reject(current, "challenge callback failed: " + conciseError(exception));
        }
    }

    private void handleHelloAck(Connection current, BridgeEnvelope envelope) {
        if (envelope.type() != BridgeMessageType.HELLO_ACK
            || envelope.sessionId() == null
            || envelope.signature() == null
            || !isFresh(envelope.sentAt(), System.currentTimeMillis())
            || !isStrictNextSequence(current.lastInboundSequence, envelope.sequence())) {
            reject(current, "invalid hello_ack headers");
            return;
        }
        byte[] c2sKey;
        byte[] s2cKey;
        try {
            c2sKey = HmacSigner.deriveSessionKey(
                config.token(),
                "c2s",
                config.serverId(),
                instanceId,
                current.clientNonce,
                current.serverNonce,
                envelope.sessionId()
            );
            s2cKey = HmacSigner.deriveSessionKey(
                config.token(),
                "s2c",
                config.serverId(),
                instanceId,
                current.clientNonce,
                current.serverNonce,
                envelope.sessionId()
            );
        } catch (RuntimeException exception) {
            reject(current, "unable to derive hello_ack session keys");
            return;
        }
        if (!HmacSigner.verify(s2cKey, envelope, "s2c")) {
            reject(current, "hello_ack signature did not verify");
            return;
        }
        BridgePayloads.HelloAck acknowledgement;
        try {
            acknowledgement = codec.parsePayload(envelope, BridgePayloads.HelloAck.class);
        } catch (ProtocolException exception) {
            reject(current, "invalid hello_ack payload");
            return;
        }
        if (!acknowledgement.accepted()
            || !current.clientNonce.equals(acknowledgement.clientNonce())
            || !current.serverNonce.equals(acknowledgement.serverNonce())) {
            reject(current, "hello_ack nonce or acceptance did not match");
            return;
        }
        current.sessionId = envelope.sessionId();
        current.c2sKey = c2sKey;
        current.s2cKey = s2cKey;
        current.lastInboundSequence = envelope.sequence();
        current.phase = ConnectionPhase.READY;
        readyEpoch.set(current.epoch);
        reconnectAttempts.set(0);
        cancelHandshakeTimeout(current);
        try {
            readyConsumer.accept(current.epoch);
        } catch (RuntimeException exception) {
            statusLogger.accept("Bridge ready callback failed: " + conciseError(exception));
        }
        scheduleOutboundDrain();
    }

    private void handleAuthenticated(Connection current, BridgeEnvelope envelope) {
        if (isHandshakeType(envelope.type())
            || !current.sessionId.equals(envelope.sessionId())
            || envelope.signature() == null
            || !isFresh(envelope.sentAt(), System.currentTimeMillis())
            || !isStrictNextSequence(current.lastInboundSequence, envelope.sequence())
            || !HmacSigner.verify(current.s2cKey, envelope, "s2c")) {
            reject(current, "invalid authenticated frame");
            return;
        }
        current.lastInboundSequence = envelope.sequence();
        try {
            inboundConsumer.accept(current.epoch, envelope);
        } catch (RuntimeException exception) {
            statusLogger.accept("Bridge inbound handler failed: " + conciseError(exception));
        }
    }

    private void reject(Connection current, String reason) {
        statusLogger.accept("Bridge rejected inbound message: " + reason);
        terminateConnection(current, "protocol rejection", true, true);
    }

    private void onSocketClosed(long epoch, WebSocket socket, String reason) {
        Connection current = connection;
        if (current != null && current.epoch == epoch && current.socket == socket) {
            terminateConnection(current, reason, true, false);
        }
    }

    private void terminateConnection(Connection current, String reason, boolean reconnect, boolean abortSocket) {
        if (connection != current) {
            return;
        }
        connection = null;
        activeSocket.compareAndSet(current.socket, null);
        activeEpoch.compareAndSet(current.epoch, 0L);
        readyEpoch.compareAndSet(current.epoch, 0L);
        awaitingHelloEpoch.compareAndSet(current.epoch, 0L);
        cancelHandshakeTimeout(current);
        current.outboundFrames.clear();
        outbound.clear();
        if (abortSocket) {
            current.socket.abort();
        }
        try {
            disconnectedConsumer.accept(current.epoch);
        } catch (RuntimeException exception) {
            statusLogger.accept("Bridge disconnected callback failed: " + conciseError(exception));
        }
        if (reconnect && isOperational()) {
            statusLogger.accept("Bridge disconnected: " + reason);
            scheduleReconnect();
        }
    }

    private void scheduleReconnect() {
        if (!isOperational()) {
            return;
        }
        int attempt = Math.min(reconnectAttempts.incrementAndGet(), 30);
        long baseSeconds = 1L << Math.min(attempt - 1, 5);
        long delaySeconds = Math.min(baseSeconds, config.network().reconnectMaxDelay().toSeconds());
        scheduleConnect(Duration.ofSeconds(delaySeconds));
    }

    private void cancelHandshakeTimeout(Connection current) {
        ScheduledFuture<?> timeout = current.handshakeTimeout;
        if (timeout != null) {
            timeout.cancel(false);
            current.handshakeTimeout = null;
        }
    }

    private boolean submitIo(Runnable task) {
        if (stopping.get()) {
            return false;
        }
        try {
            ioExecutor.execute(task);
            return true;
        } catch (RejectedExecutionException exception) {
            return false;
        }
    }

    private static boolean isFresh(long sentAt, long now) {
        return sentAt >= now - MAX_CLOCK_SKEW_MILLIS && sentAt <= now + MAX_CLOCK_SKEW_MILLIS;
    }

    private static boolean isStrictNextSequence(long previous, long next) {
        return next > previous && next - previous <= MAX_SEQUENCE_GAP;
    }

    private static boolean isHandshakeType(BridgeMessageType type) {
        return type == BridgeMessageType.HELLO_INIT
            || type == BridgeMessageType.CHALLENGE
            || type == BridgeMessageType.HELLO
            || type == BridgeMessageType.HELLO_ACK;
    }

    private static boolean validNonce(String nonce) {
        if (nonce == null || nonce.isBlank() || nonce.indexOf('=') >= 0 || nonce.length() % 4 == 1) {
            return false;
        }
        for (int index = 0; index < nonce.length(); index++) {
            char character = nonce.charAt(index);
            boolean allowed = (character >= 'A' && character <= 'Z')
                || (character >= 'a' && character <= 'z')
                || (character >= '0' && character <= '9')
                || character == '-'
                || character == '_';
            if (!allowed) {
                return false;
            }
        }
        try {
            return Base64.getUrlDecoder().decode(nonce).length == NONCE_BYTES;
        } catch (IllegalArgumentException exception) {
            return false;
        }
    }

    private static String createNonce() {
        byte[] bytes = new byte[NONCE_BYTES];
        NonceHolder.RANDOM.nextBytes(bytes);
        return Base64.getUrlEncoder().withoutPadding().encodeToString(bytes);
    }

    private static String requireNonBlank(String value, String field) {
        if (value == null || value.isBlank()) {
            throw new IllegalArgumentException(field + " must not be blank");
        }
        return value;
    }

    private static ThreadFactory namedThreadFactory(String prefix) {
        return runnable -> {
            Thread thread = new Thread(runnable, prefix);
            thread.setDaemon(true);
            return thread;
        };
    }

    private static String conciseError(Throwable error) {
        String message = error.getMessage();
        return error.getClass().getSimpleName() + (message == null || message.isBlank() ? "" : ": " + message);
    }

    public record HandshakeChallenge(long epoch, String clientNonce, String serverNonce) {
        public HandshakeChallenge {
            if (epoch < 1) {
                throw new IllegalArgumentException("epoch must be positive");
            }
            requireNonBlank(clientNonce, "clientNonce");
            requireNonBlank(serverNonce, "serverNonce");
        }
    }

    private record OutboundIntent(long epoch, BridgeMessageType type, String requestId, JsonObject payload) {
    }

    private enum ConnectionPhase {
        WAITING_CHALLENGE,
        WAITING_HELLO,
        WAITING_ACK,
        READY
    }

    private static final class Connection {
        private final long epoch;
        private final WebSocket socket;
        private final ProtocolFactory factory;
        private final ArrayDeque<String> outboundFrames = new ArrayDeque<>();
        private ConnectionPhase phase = ConnectionPhase.WAITING_CHALLENGE;
        private String clientNonce;
        private String serverNonce;
        private long challengeExpiresAt;
        private String sessionId;
        private byte[] c2sKey;
        private byte[] s2cKey;
        private long lastInboundSequence;
        private boolean sendInFlight;
        private ScheduledFuture<?> handshakeTimeout;

        private Connection(long epoch, WebSocket socket, ProtocolFactory factory) {
            this.epoch = epoch;
            this.socket = socket;
            this.factory = factory;
        }
    }

    private final class Listener implements WebSocket.Listener {
        private final long epoch;
        private final StringBuilder messageBuffer = new StringBuilder();

        private Listener(long epoch) {
            this.epoch = epoch;
        }

        @Override
        public void onOpen(WebSocket webSocket) {
            webSocket.request(1);
            if (!submitIo(() -> activateConnection(epoch, webSocket))) {
                webSocket.abort();
            }
        }

        @Override
        public CompletionStage<?> onText(WebSocket webSocket, CharSequence data, boolean last) {
            synchronized (messageBuffer) {
                if (messageBuffer.length() + data.length() > MAX_INBOUND_CHARS) {
                    webSocket.abort();
                    submitIo(() -> onSocketClosed(epoch, webSocket, "inbound message exceeded size limit"));
                    return CompletableFuture.completedFuture(null);
                }
                messageBuffer.append(data);
                if (last) {
                    String completeMessage = messageBuffer.toString();
                    messageBuffer.setLength(0);
                    submitIo(() -> handleMessage(epoch, webSocket, completeMessage));
                }
            }
            webSocket.request(1);
            return CompletableFuture.completedFuture(null);
        }

        @Override
        public CompletionStage<?> onBinary(WebSocket webSocket, ByteBuffer data, boolean last) {
            webSocket.abort();
            submitIo(() -> onSocketClosed(epoch, webSocket, "unexpected binary message"));
            return CompletableFuture.completedFuture(null);
        }

        @Override
        public CompletionStage<?> onClose(WebSocket webSocket, int statusCode, String reason) {
            submitIo(() -> onSocketClosed(epoch, webSocket, "close " + statusCode));
            return CompletableFuture.completedFuture(null);
        }

        @Override
        public void onError(WebSocket webSocket, Throwable error) {
            submitIo(() -> onSocketClosed(epoch, webSocket, "network error: " + conciseError(error)));
        }
    }

    private static final class NonceHolder {
        private static final java.security.SecureRandom RANDOM = new java.security.SecureRandom();

        private NonceHolder() {
        }
    }
}
