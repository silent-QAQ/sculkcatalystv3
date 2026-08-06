package com.sculkcatalyst.paperbridge.protocol;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.util.Base64;
import java.util.Objects;
import javax.crypto.Mac;
import javax.crypto.spec.SecretKeySpec;

/** Canonical HMAC-SHA-256 helpers for the v2 handshake and session frames. */
public final class HmacSigner {
    private static final String HMAC_SHA_256 = "HmacSHA256";
    private static final Base64.Encoder BASE64_URL = Base64.getUrlEncoder().withoutPadding();

    private HmacSigner() {
    }

    public static String sign(String token, BridgeEnvelope envelope, String direction) {
        Objects.requireNonNull(token, "token");
        return sign(token.getBytes(StandardCharsets.UTF_8), envelope, direction);
    }

    public static String sign(byte[] key, BridgeEnvelope envelope, String direction) {
        return BASE64_URL.encodeToString(hmac(key, canonicalString(envelope, direction).getBytes(StandardCharsets.UTF_8)));
    }

    public static boolean verify(String token, BridgeEnvelope envelope, String direction) {
        Objects.requireNonNull(token, "token");
        return verify(token.getBytes(StandardCharsets.UTF_8), envelope, direction);
    }

    public static boolean verify(byte[] key, BridgeEnvelope envelope, String direction) {
        if (envelope.signature() == null) {
            return false;
        }
        byte[] expected = sign(key, envelope.withSignature(null), direction).getBytes(StandardCharsets.US_ASCII);
        byte[] actual = envelope.signature().getBytes(StandardCharsets.US_ASCII);
        return MessageDigest.isEqual(expected, actual);
    }

    public static byte[] deriveSessionKey(
        String token,
        String direction,
        String serverId,
        String instanceId,
        String clientNonce,
        String serverNonce,
        String sessionId
    ) {
        if (!("c2s".equals(direction) || "s2c".equals(direction))) {
            throw new IllegalArgumentException("direction must be c2s or s2c");
        }
        requireNonBlank(token, "token");
        requireNonBlank(serverId, "serverId");
        requireNonBlank(instanceId, "instanceId");
        requireNonBlank(clientNonce, "clientNonce");
        requireNonBlank(serverNonce, "serverNonce");
        requireNonBlank(sessionId, "sessionId");
        String canonical = "sculk-catalyst-bridge-v2\n"
            + "direction=" + direction + '\n'
            + "server_id=" + base64UrlUtf8(serverId) + '\n'
            + "instance_id=" + base64UrlUtf8(instanceId) + '\n'
            + "client_nonce=" + clientNonce + '\n'
            + "server_nonce=" + serverNonce + '\n'
            + "session_id=" + base64UrlUtf8(sessionId);
        return hmac(token.getBytes(StandardCharsets.UTF_8), canonical.getBytes(StandardCharsets.UTF_8));
    }

    public static String canonicalString(BridgeEnvelope envelope, String direction) {
        Objects.requireNonNull(envelope, "envelope");
        requireNonBlank(direction, "direction");
        String payloadHash = BASE64_URL.encodeToString(sha256(envelope.payloadJsonBytes()));
        return "protocol_version=" + envelope.protocolVersion() + '\n'
            + "direction=" + direction + '\n'
            + "type=" + envelope.type().wireName() + '\n'
            + "request_id=" + nullableBase64UrlUtf8(envelope.requestId()) + '\n'
            + "server_id=" + base64UrlUtf8(envelope.serverId()) + '\n'
            + "instance_id=" + base64UrlUtf8(envelope.instanceId()) + '\n'
            + "session_id=" + nullableBase64UrlUtf8(envelope.sessionId()) + '\n'
            + "seq=" + envelope.sequence() + '\n'
            + "sent_at=" + envelope.sentAt() + '\n'
            + "payload_sha256=" + payloadHash;
    }

    private static byte[] hmac(byte[] key, byte[] content) {
        try {
            Mac mac = Mac.getInstance(HMAC_SHA_256);
            mac.init(new SecretKeySpec(key, HMAC_SHA_256));
            return mac.doFinal(content);
        } catch (GeneralSecurityException exception) {
            throw new IllegalStateException("Unable to calculate bridge signature", exception);
        }
    }

    private static byte[] sha256(byte[] content) {
        try {
            return MessageDigest.getInstance("SHA-256").digest(content);
        } catch (GeneralSecurityException exception) {
            throw new IllegalStateException("SHA-256 is unavailable", exception);
        }
    }

    private static String nullableBase64UrlUtf8(String value) {
        return value == null || value.isEmpty() ? "-" : base64UrlUtf8(value);
    }

    private static String base64UrlUtf8(String value) {
        return BASE64_URL.encodeToString(value.getBytes(StandardCharsets.UTF_8));
    }

    private static void requireNonBlank(String value, String field) {
        if (value == null || value.isBlank()) {
            throw new IllegalArgumentException(field + " must not be blank");
        }
    }
}
