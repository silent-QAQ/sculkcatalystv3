package com.sculkcatalyst.paperbridge.protocol;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.List;
import org.junit.jupiter.api.Test;

class ProtocolCodecTest {
    private final ProtocolCodec codec = new ProtocolCodec();

    @Test
    void roundTripsTheStrictV2EnvelopeAndEncodedPayload() {
        JsonObject payload = new JsonObject();
        payload.addProperty("player_uuid", "b54cf3f8-9f51-4c4c-a3d1-9584fce2822c");
        BridgeEnvelope envelope = envelope(
            BridgeMessageType.SNAPSHOT_REQUEST,
            "req-1",
            "session-a",
            7,
            1_725_000_000_000L,
            payload,
            null
        );

        String encoded = codec.encode(envelope);
        JsonObject wire = JsonParser.parseString(encoded).getAsJsonObject();
        assertEquals(10, wire.size());
        assertEquals(2, wire.get("protocol_version").getAsInt());
        assertEquals("snapshot_request", wire.get("type").getAsString());
        assertEquals("req-1", wire.get("request_id").getAsString());
        assertEquals("session-a", wire.get("session_id").getAsString());
        assertEquals(7, wire.get("seq").getAsInt());
        assertFalse(wire.has("payload"));
        assertArrayEquals(
            envelope.payloadJsonBytes(),
            Base64.getUrlDecoder().decode(wire.get("payload_json").getAsString())
        );

        BridgeEnvelope decoded = codec.decode(encoded);
        assertEquals(envelope.protocolVersion(), decoded.protocolVersion());
        assertEquals(envelope.type(), decoded.type());
        assertEquals(envelope.requestId(), decoded.requestId());
        assertEquals(envelope.serverId(), decoded.serverId());
        assertEquals(envelope.instanceId(), decoded.instanceId());
        assertEquals(envelope.sessionId(), decoded.sessionId());
        assertArrayEquals(envelope.payloadJsonBytes(), decoded.payloadJsonBytes());
        assertEquals("b54cf3f8-9f51-4c4c-a3d1-9584fce2822c", decoded.payload().get("player_uuid").getAsString());
    }

    @Test
    void preservesOriginalPayloadBytesInsteadOfReserializingThem() {
        byte[] rawPayload = "{\"player_uuid\": \"b54cf3f8-9f51-4c4c-a3d1-9584fce2822c\", \"nested\":{\"a\":1}}"
            .getBytes(StandardCharsets.UTF_8);
        String payloadJson = Base64.getUrlEncoder().withoutPadding().encodeToString(rawPayload);
        String wire = """
            {"protocol_version":2,"type":"snapshot_request","request_id":"req-1","server_id":"server-a","instance_id":"instance-a","session_id":"session-a","seq":7,"sent_at":1725000000000,"payload_json":"%s","signature":null}
            """.formatted(payloadJson);

        BridgeEnvelope decoded = codec.decode(wire);
        assertArrayEquals(rawPayload, decoded.payloadJsonBytes());
        assertEquals(1, decoded.payload().getAsJsonObject("nested").get("a").getAsInt());
    }

    @Test
    void encodesNullableV2HeaderFieldsExplicitly() {
        BridgeEnvelope envelope = envelope(
            BridgeMessageType.HELLO_INIT,
            null,
            null,
            1,
            1,
            new JsonObject(),
            null
        );

        JsonObject wire = JsonParser.parseString(codec.encode(envelope)).getAsJsonObject();
        assertTrue(wire.get("request_id").isJsonNull());
        assertTrue(wire.get("session_id").isJsonNull());
        assertTrue(wire.get("signature").isJsonNull());
        assertNull(codec.decode(codec.encode(envelope)).requestId());
    }

    @Test
    void rejectsMissingExtraOrFractionalEnvelopeFields() {
        JsonObject payload = new JsonObject();
        BridgeEnvelope envelope = envelope(
            BridgeMessageType.HEARTBEAT,
            null,
            "session-a",
            1,
            1,
            payload,
            "c2lnbmF0dXJl"
        );
        JsonObject missing = JsonParser.parseString(codec.encode(envelope)).getAsJsonObject();
        missing.remove("signature");
        assertThrows(ProtocolException.class, () -> codec.decode(missing.toString()));

        JsonObject extra = JsonParser.parseString(codec.encode(envelope)).getAsJsonObject();
        extra.addProperty("unexpected", true);
        assertThrows(ProtocolException.class, () -> codec.decode(extra.toString()));

        JsonObject fractional = JsonParser.parseString(codec.encode(envelope)).getAsJsonObject();
        fractional.addProperty("seq", 1.5D);
        assertThrows(ProtocolException.class, () -> codec.decode(fractional.toString()));
    }

    @Test
    void serializesHelloWithoutTheSecret() {
        JsonObject payload = codec.payloadOf(new BridgePayloads.Hello(
            "Y2xpZW50LW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            "c2VydmVyLW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            List.of("snapshot", "papi_read"),
            "paper-1.21.6"
        ));

        assertEquals("Y2xpZW50LW5vbmNlLTAxMjM0NTY3ODkwMTIz", payload.get("client_nonce").getAsString());
        assertEquals("paper-1.21.6", payload.get("runtime_generation").getAsString());
        assertFalse(payload.has("authentication"));
        assertFalse(payload.has("token"));
        assertFalse(payload.has("signature"));
    }

    @Test
    void serializesPapiRequestsAsIdentifierAndPlaceholderPairs() {
        JsonObject payload = codec.payloadOf(new BridgePayloads.PapiRequest(
            "b54cf3f8-9f51-4c4c-a3d1-9584fce2822c",
            List.of(new BridgePayloads.PapiRequestField("a-field-id", "%vault_eco_balance%"))
        ));

        JsonArray fields = payload.getAsJsonArray("fields");
        assertEquals(1, fields.size());
        assertEquals("a-field-id", fields.get(0).getAsJsonObject().get("id").getAsString());
        assertEquals("%vault_eco_balance%", fields.get(0).getAsJsonObject().get("placeholder").getAsString());
    }

    private BridgeEnvelope envelope(
        BridgeMessageType type,
        String requestId,
        String sessionId,
        long sequence,
        long sentAt,
        JsonObject payload,
        String signature
    ) {
        return new BridgeEnvelope(
            ProtocolCodec.PROTOCOL_VERSION,
            type,
            requestId,
            "server-a",
            "instance-a",
            sessionId,
            sequence,
            sentAt,
            codec.payloadBytesOf(payload),
            payload,
            signature
        );
    }
}
