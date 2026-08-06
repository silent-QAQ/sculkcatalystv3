package com.sculkcatalyst.paperbridge.protocol;

import com.google.gson.JsonObject;
import java.util.Arrays;
import java.util.Objects;

/**
 * An immutable v2 bridge frame after its Base64URL payload has been decoded.
 *
 * <p>The original UTF-8 payload bytes are retained because the signature covers their SHA-256
 * digest, rather than a re-serialized JSON representation.</p>
 */
public record BridgeEnvelope(
    int protocolVersion,
    BridgeMessageType type,
    String requestId,
    String serverId,
    String instanceId,
    String sessionId,
    long sequence,
    long sentAt,
    byte[] payloadJsonBytes,
    JsonObject payload,
    String signature
) {
    public BridgeEnvelope {
        Objects.requireNonNull(type, "type");
        requireNonBlank(serverId, "serverId");
        requireNonBlank(instanceId, "instanceId");
        if (sessionId != null && sessionId.isBlank()) {
            throw new IllegalArgumentException("sessionId must be null or non-blank");
        }
        if (sequence < 1 || sentAt < 1) {
            throw new IllegalArgumentException("sequence and sentAt must be positive");
        }
        Objects.requireNonNull(payloadJsonBytes, "payloadJsonBytes");
        Objects.requireNonNull(payload, "payload");
        if (signature != null && signature.isBlank()) {
            throw new IllegalArgumentException("signature must be null or non-blank");
        }
        payloadJsonBytes = Arrays.copyOf(payloadJsonBytes, payloadJsonBytes.length);
        payload = payload.deepCopy();
    }

    @Override
    public byte[] payloadJsonBytes() {
        return Arrays.copyOf(payloadJsonBytes, payloadJsonBytes.length);
    }

    @Override
    public JsonObject payload() {
        return payload.deepCopy();
    }

    public BridgeEnvelope withSignature(String newSignature) {
        return new BridgeEnvelope(
            protocolVersion,
            type,
            requestId,
            serverId,
            instanceId,
            sessionId,
            sequence,
            sentAt,
            payloadJsonBytes,
            payload,
            newSignature
        );
    }

    private static void requireNonBlank(String value, String field) {
        if (value == null || value.isBlank()) {
            throw new IllegalArgumentException(field + " must not be blank");
        }
    }
}
