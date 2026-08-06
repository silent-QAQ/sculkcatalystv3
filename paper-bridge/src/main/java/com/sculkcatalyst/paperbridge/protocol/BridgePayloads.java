package com.sculkcatalyst.paperbridge.protocol;

import com.google.gson.JsonObject;
import java.util.List;

/** Typed payload DTOs. These records intentionally contain no Bukkit objects. */
public final class BridgePayloads {
    private BridgePayloads() {
    }

    public record HelloInit(String clientNonce) {
    }

    public record Challenge(String clientNonce, String serverNonce, long expiresAt) {
    }

    public record Hello(String clientNonce, String serverNonce, List<String> capabilities, String runtimeGeneration) {
    }

    public record HelloAck(boolean accepted, String clientNonce, String serverNonce, List<String> capabilities) {
    }

    public record Heartbeat(long uptimeMs, int onlineCount, boolean helloAcknowledged) {
    }

    public record PresenceSync(String reason, List<JsonObject> players, boolean complete) {
    }

    public record PlayerDelta(String action, JsonObject player) {
    }

    public record SnapshotRequest(String playerUuid, List<String> sections) {
    }

    public record SnapshotResponse(String status, String playerUuid, JsonObject snapshot, String errorCode) {
    }

    public record PapiRequest(String playerUuid, List<PapiRequestField> fields) {
    }

    public record PapiRequestField(String id, String placeholder) {
    }

    public record PapiResponse(String status, String playerUuid, JsonObject fields, String errorCode) {
    }

    public record Error(String code, String message) {
    }

    public record Bye(String reason) {
    }
}
