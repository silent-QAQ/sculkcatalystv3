package com.sculkcatalyst.paperbridge.protocol;

import com.google.gson.JsonObject;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicLong;

/** Adds invariant v2 fields and a connection-local sequence to outbound frames. */
public final class ProtocolFactory {
    private final String serverId;
    private final String instanceId;
    private final ProtocolCodec codec;
    private final AtomicLong sequence = new AtomicLong();

    public ProtocolFactory(String serverId, String instanceId, ProtocolCodec codec) {
        this.serverId = requireNonBlank(serverId, "serverId");
        this.instanceId = requireNonBlank(instanceId, "instanceId");
        this.codec = Objects.requireNonNull(codec, "codec");
    }

    public BridgeEnvelope createUnsigned(BridgeMessageType type, String requestId, String sessionId, Object payload) {
        return create(type, requestId, sessionId, codec.payloadOf(payload));
    }

    public BridgeEnvelope createSigned(
        BridgeMessageType type,
        String requestId,
        String sessionId,
        JsonObject payload,
        byte[] key,
        String direction
    ) {
        BridgeEnvelope unsigned = create(type, requestId, sessionId, payload);
        return unsigned.withSignature(HmacSigner.sign(key, unsigned, direction));
    }

    public BridgeEnvelope createSigned(
        BridgeMessageType type,
        String requestId,
        String sessionId,
        Object payload,
        String token,
        String direction
    ) {
        BridgeEnvelope unsigned = createUnsigned(type, requestId, sessionId, payload);
        return unsigned.withSignature(HmacSigner.sign(token, unsigned, direction));
    }

    private BridgeEnvelope create(BridgeMessageType type, String requestId, String sessionId, JsonObject payload) {
        JsonObject payloadCopy = Objects.requireNonNull(payload, "payload").deepCopy();
        return new BridgeEnvelope(
            ProtocolCodec.PROTOCOL_VERSION,
            Objects.requireNonNull(type, "type"),
            requestId,
            serverId,
            instanceId,
            sessionId,
            sequence.incrementAndGet(),
            System.currentTimeMillis(),
            codec.payloadBytesOf(payloadCopy),
            payloadCopy,
            null
        );
    }

    private static String requireNonBlank(String value, String field) {
        if (value == null || value.isBlank()) {
            throw new IllegalArgumentException(field + " must not be blank");
        }
        return value;
    }
}
