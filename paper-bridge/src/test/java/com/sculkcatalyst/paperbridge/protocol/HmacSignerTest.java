package com.sculkcatalyst.paperbridge.protocol;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import com.google.gson.JsonObject;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import org.junit.jupiter.api.Test;

class HmacSignerTest {
    private static final String TOKEN = "an-example-secret-token";

    @Test
    void signsAndVerifiesTheDocumentedV2CanonicalForm() {
        BridgeEnvelope envelope = envelope("b54cf3f8-9f51-4c4c-a3d1-9584fce2822c");

        assertEquals(
            "protocol_version=2\n"
                + "direction=c2s\n"
                + "type=snapshot_request\n"
                + "request_id=cmVxLTQy\n"
                + "server_id=c2VydmVyLWE\n"
                + "instance_id=aW5zdGFuY2UtYQ\n"
                + "session_id=c2Vzc2lvbi1h\n"
                + "seq=7\n"
                + "sent_at=1725000000000\n"
                + "payload_sha256=_EaLh-ZpGS1szc1BW_QlBkBYlRTvN2uiNb2dYfwB3Qk",
            HmacSigner.canonicalString(envelope, "c2s")
        );

        String signature = HmacSigner.sign(TOKEN, envelope, "c2s");
        assertEquals("VY3g3hic8F-GrBVsE6VUvu2CTEh2LEzPdDj9Bho7z34", signature);
        assertTrue(HmacSigner.verify(TOKEN, envelope.withSignature(signature), "c2s"));
        assertFalse(HmacSigner.verify(TOKEN, envelope.withSignature(signature), "s2c"));
        assertFalse(HmacSigner.verify(TOKEN, envelope("other-player").withSignature(signature), "c2s"));
        assertFalse(HmacSigner.verify(TOKEN, envelope, "c2s"));
    }

    @Test
    void derivesIndependentDirectionalSessionKeys() {
        byte[] clientToServer = HmacSigner.deriveSessionKey(
            TOKEN,
            "c2s",
            "server-a",
            "instance-a",
            "Y2xpZW50LW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            "c2VydmVyLW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            "session-a"
        );
        byte[] serverToClient = HmacSigner.deriveSessionKey(
            TOKEN,
            "s2c",
            "server-a",
            "instance-a",
            "Y2xpZW50LW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            "c2VydmVyLW5vbmNlLTAxMjM0NTY3ODkwMTIz",
            "session-a"
        );

        assertFalse(Arrays.equals(clientToServer, serverToClient));
        assertTrue(HmacSigner.verify(clientToServer, envelope("player").withSignature(
            HmacSigner.sign(clientToServer, envelope("player"), "c2s")
        ), "c2s"));
    }

    private static BridgeEnvelope envelope(String playerUuid) {
        JsonObject payload = new JsonObject();
        payload.addProperty("player_uuid", playerUuid);
        byte[] payloadBytes = payload.toString().getBytes(StandardCharsets.UTF_8);
        return new BridgeEnvelope(
            ProtocolCodec.PROTOCOL_VERSION,
            BridgeMessageType.SNAPSHOT_REQUEST,
            "req-42",
            "server-a",
            "instance-a",
            "session-a",
            7,
            1_725_000_000_000L,
            payloadBytes,
            payload,
            null
        );
    }
}
