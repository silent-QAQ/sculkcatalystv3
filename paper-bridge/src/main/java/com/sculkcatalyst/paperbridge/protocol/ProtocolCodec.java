package com.sculkcatalyst.paperbridge.protocol;

import com.google.gson.FieldNamingPolicy;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.nio.ByteBuffer;
import java.nio.CharBuffer;
import java.nio.charset.CharacterCodingException;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.Objects;
import java.util.Set;

/** JSON codec with strict v2 envelope validation before a message reaches Bukkit code. */
public final class ProtocolCodec {
    public static final int PROTOCOL_VERSION = 2;

    private static final Set<String> FRAME_FIELDS = Set.of(
        "protocol_version",
        "type",
        "request_id",
        "server_id",
        "instance_id",
        "session_id",
        "seq",
        "sent_at",
        "payload_json",
        "signature"
    );

    private final Gson gson = new GsonBuilder()
        .disableHtmlEscaping()
        .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES)
        .create();

    public String encode(BridgeEnvelope envelope) {
        Objects.requireNonNull(envelope, "envelope");
        JsonObject json = new JsonObject();
        json.addProperty("protocol_version", envelope.protocolVersion());
        json.addProperty("type", envelope.type().wireName());
        addNullableString(json, "request_id", envelope.requestId());
        json.addProperty("server_id", envelope.serverId());
        json.addProperty("instance_id", envelope.instanceId());
        addNullableString(json, "session_id", envelope.sessionId());
        json.addProperty("seq", envelope.sequence());
        json.addProperty("sent_at", envelope.sentAt());
        json.addProperty(
            "payload_json",
            Base64.getUrlEncoder().withoutPadding().encodeToString(envelope.payloadJsonBytes())
        );
        addNullableString(json, "signature", envelope.signature());
        return json.toString();
    }

    public BridgeEnvelope decode(String rawJson) {
        try {
            JsonElement root = JsonParser.parseString(rawJson);
            if (!root.isJsonObject()) {
                throw new ProtocolException("Bridge message must be a JSON object");
            }
            JsonObject json = root.getAsJsonObject();
            if (json.size() != FRAME_FIELDS.size() || !FRAME_FIELDS.equals(json.keySet())) {
                throw new ProtocolException("Bridge message must contain exactly the v2 envelope fields");
            }
            int protocolVersion = requiredInt(json, "protocol_version");
            if (protocolVersion != PROTOCOL_VERSION) {
                throw new ProtocolException("Unsupported protocol_version: " + protocolVersion);
            }
            String rawType = requiredString(json, "type");
            BridgeMessageType type = BridgeMessageType.fromWireName(rawType)
                .orElseThrow(() -> new ProtocolException("Unsupported message type: " + rawType));
            String requestId = nullableString(json, "request_id");
            String serverId = requiredString(json, "server_id");
            String instanceId = requiredString(json, "instance_id");
            String sessionId = nullableString(json, "session_id");
            long sequence = requiredLong(json, "seq");
            long sentAt = requiredLong(json, "sent_at");
            if (sequence < 1 || sentAt < 1) {
                throw new ProtocolException("seq and sent_at must be positive");
            }
            String payloadJson = requiredBase64Url(json, "payload_json");
            byte[] payloadBytes = decodeBase64Url(payloadJson, "payload_json");
            JsonObject payload = decodePayloadObject(payloadBytes);
            String signature = nullableBase64Url(json, "signature");
            return new BridgeEnvelope(
                protocolVersion,
                type,
                requestId,
                serverId,
                instanceId,
                sessionId,
                sequence,
                sentAt,
                payloadBytes,
                payload,
                signature
            );
        } catch (ProtocolException exception) {
            throw exception;
        } catch (RuntimeException exception) {
            throw new ProtocolException("Malformed bridge JSON", exception);
        }
    }

    public JsonObject payloadOf(Object payload) {
        JsonElement tree = gson.toJsonTree(Objects.requireNonNull(payload, "payload"));
        if (!tree.isJsonObject()) {
            throw new IllegalArgumentException("Bridge payload must serialize to a JSON object");
        }
        return tree.getAsJsonObject();
    }

    public byte[] payloadBytesOf(JsonObject payload) {
        Objects.requireNonNull(payload, "payload");
        return gson.toJson(payload).getBytes(StandardCharsets.UTF_8);
    }

    public <T> T parsePayload(BridgeEnvelope envelope, Class<T> payloadType) {
        Objects.requireNonNull(envelope, "envelope");
        Objects.requireNonNull(payloadType, "payloadType");
        try {
            return gson.fromJson(envelope.payload(), payloadType);
        } catch (RuntimeException exception) {
            throw new ProtocolException("Invalid " + envelope.type().wireName() + " payload", exception);
        }
    }

    private static void addNullableString(JsonObject object, String field, String value) {
        if (value == null) {
            object.add(field, JsonNull.INSTANCE);
        } else {
            object.addProperty(field, value);
        }
    }

    private static JsonObject decodePayloadObject(byte[] payloadBytes) {
        String payloadText;
        try {
            payloadText = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(payloadBytes))
                .toString();
        } catch (CharacterCodingException exception) {
            throw new ProtocolException("payload_json must decode to valid UTF-8", exception);
        }
        JsonElement payload;
        try {
            payload = JsonParser.parseString(payloadText);
        } catch (RuntimeException exception) {
            throw new ProtocolException("payload_json must decode to JSON", exception);
        }
        if (!payload.isJsonObject()) {
            throw new ProtocolException("payload_json must decode to a JSON object");
        }
        return payload.getAsJsonObject();
    }

    private static String requiredString(JsonObject object, String field) {
        String value = nullableString(object, field);
        if (value == null || value.isBlank()) {
            throw new ProtocolException(field + " must be a non-blank string");
        }
        return value;
    }

    private static String nullableString(JsonObject object, String field) {
        JsonElement value = object.get(field);
        if (value == null) {
            throw new ProtocolException(field + " must be present");
        }
        if (value.isJsonNull()) {
            return null;
        }
        if (!value.isJsonPrimitive() || !value.getAsJsonPrimitive().isString()) {
            throw new ProtocolException(field + " must be a string or null");
        }
        String text = value.getAsString();
        if (text.isBlank()) {
            throw new ProtocolException(field + " must not be blank when present");
        }
        return text;
    }

    private static String requiredBase64Url(JsonObject object, String field) {
        String value = requiredString(object, field);
        validateBase64Url(value, field);
        return value;
    }

    private static String nullableBase64Url(JsonObject object, String field) {
        String value = nullableString(object, field);
        if (value != null) {
            validateBase64Url(value, field);
        }
        return value;
    }

    private static byte[] decodeBase64Url(String value, String field) {
        try {
            return Base64.getUrlDecoder().decode(value);
        } catch (IllegalArgumentException exception) {
            throw new ProtocolException(field + " must be Base64URL without padding", exception);
        }
    }

    private static void validateBase64Url(String value, String field) {
        if (value.indexOf('=') >= 0 || value.length() % 4 == 1) {
            throw new ProtocolException(field + " must be unpadded Base64URL");
        }
        for (int index = 0; index < value.length(); index++) {
            char character = value.charAt(index);
            boolean allowed = (character >= 'A' && character <= 'Z')
                || (character >= 'a' && character <= 'z')
                || (character >= '0' && character <= '9')
                || character == '-'
                || character == '_';
            if (!allowed) {
                throw new ProtocolException(field + " must be unpadded Base64URL");
            }
        }
    }

    private static int requiredInt(JsonObject object, String field) {
        JsonElement value = requiredNumber(object, field);
        try {
            return value.getAsBigDecimal().intValueExact();
        } catch (NumberFormatException | ArithmeticException exception) {
            throw new ProtocolException(field + " must be an integer", exception);
        }
    }

    private static long requiredLong(JsonObject object, String field) {
        JsonElement value = requiredNumber(object, field);
        try {
            return value.getAsBigDecimal().longValueExact();
        } catch (NumberFormatException | ArithmeticException exception) {
            throw new ProtocolException(field + " must be an integer", exception);
        }
    }

    private static JsonElement requiredNumber(JsonObject object, String field) {
        JsonElement value = object.get(field);
        if (value == null || !value.isJsonPrimitive() || !value.getAsJsonPrimitive().isNumber()) {
            throw new ProtocolException(field + " must be a number");
        }
        return value;
    }
}
