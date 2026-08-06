package com.sculkcatalyst.paperbridge;

import com.google.gson.JsonObject;
import com.sculkcatalyst.paperbridge.client.BridgeWebSocketClient;
import com.sculkcatalyst.paperbridge.config.BridgeConfig;
import com.sculkcatalyst.paperbridge.config.ConfigException;
import com.sculkcatalyst.paperbridge.papi.PapiResolver;
import com.sculkcatalyst.paperbridge.papi.PapiResolvers;
import com.sculkcatalyst.paperbridge.platform.FoliaTaskDispatcher;
import com.sculkcatalyst.paperbridge.player.ItemSerializer;
import com.sculkcatalyst.paperbridge.player.PlayerSnapshotService;
import com.sculkcatalyst.paperbridge.protocol.BridgeEnvelope;
import com.sculkcatalyst.paperbridge.protocol.BridgeMessageType;
import com.sculkcatalyst.paperbridge.protocol.BridgePayloads;
import com.sculkcatalyst.paperbridge.protocol.ProtocolCodec;
import com.sculkcatalyst.paperbridge.text.Utf8TextLimiter;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicInteger;
import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerQuitEvent;
import org.bukkit.plugin.java.JavaPlugin;

/**
 * Read-only player data bridge. This plugin never executes commands and never mutates player
 * state. Network authentication is completed before this class schedules any Bukkit work.
 */
public final class PaperBridgePlugin extends JavaPlugin implements Listener {
    private static final int MAX_PAPI_REQUEST_FIELDS = 10;
    private static final int MAX_PAPI_PLACEHOLDER_LENGTH = 256;
    private static final int MAX_SNAPSHOT_SECTIONS = 3;
    private static final int MAX_PRESENCE_BATCH = 500;
    // Keeps the Base64URL v2 wire frame under the backend's 512 KiB frame limit.
    private static final int MAX_SNAPSHOT_RESPONSE_PAYLOAD_BYTES = 350 * 1024;
    private static final Set<String> ALLOWED_SNAPSHOT_SECTIONS = Set.of("basic", "inventory", "ender_chest");

    private final AtomicBoolean stopping = new AtomicBoolean();

    private BridgeConfig bridgeConfig;
    private FoliaTaskDispatcher tasks;
    private ProtocolCodec protocolCodec;
    private BridgeWebSocketClient bridgeClient;
    private PlayerSnapshotService snapshotService;
    private PapiResolver papiResolver;
    private UUID instanceId;
    private long startedAt;

    @Override
    public void onEnable() {
        saveDefaultConfig();
        try {
            bridgeConfig = BridgeConfig.load(getConfig());
        } catch (ConfigException exception) {
            getLogger().severe("Invalid bridge configuration: " + exception.getMessage());
            getServer().getPluginManager().disablePlugin(this);
            return;
        }
        if (!bridgeConfig.enabled()) {
            getLogger().info("Bridge is disabled by configuration.");
            return;
        }

        stopping.set(false);
        instanceId = UUID.randomUUID();
        startedAt = System.currentTimeMillis();
        protocolCodec = new ProtocolCodec();
        tasks = new FoliaTaskDispatcher(this);
        snapshotService = new PlayerSnapshotService(new ItemSerializer(bridgeConfig.snapshot()));
        papiResolver = PapiResolvers.create(this, bridgeConfig.papi().enabled());
        bridgeClient = new BridgeWebSocketClient(
            bridgeConfig,
            instanceId.toString(),
            protocolCodec,
            this::handleInbound,
            this::onHandshakeChallenge,
            this::onBridgeReady,
            this::onBridgeDisconnected,
            message -> getLogger().warning(message)
        );

        getServer().getPluginManager().registerEvents(this, this);
        tasks.runGlobalAtFixedRate(
            ignored -> sendHeartbeat(),
            20L,
            bridgeConfig.network().heartbeatSeconds() * 20L
        );
        bridgeClient.start();
        getLogger().info("Read-only bridge starting for server-id " + bridgeConfig.serverId() + ".");
    }

    @Override
    public void onDisable() {
        stopping.set(true);
        if (bridgeClient != null) {
            bridgeClient.close();
        }
        if (tasks != null) {
            tasks.cancelOwnedGlobalTasks();
        }
    }

    @EventHandler
    public void onPlayerJoin(PlayerJoinEvent event) {
        long epoch = readyEpoch();
        if (!isReady(epoch)) {
            return;
        }
        tasks.runForPlayer(event.getPlayer(), player -> {
            if (isReady(epoch)) {
                send(epoch, BridgeMessageType.PLAYER_DELTA, null, new BridgePayloads.PlayerDelta("join", snapshotService.basicPlayer(player)));
            }
        }, () -> {
            // The next full presence sync will reconcile a player retired during a join event.
        });
    }

    @EventHandler
    public void onPlayerQuit(PlayerQuitEvent event) {
        long epoch = readyEpoch();
        if (!isReady(epoch)) {
            return;
        }
        Player eventPlayer = event.getPlayer();
        String playerId = eventPlayer.getUniqueId().toString();
        String name = eventPlayer.getName();
        tasks.runForPlayer(eventPlayer, player -> {
            if (!isReady(epoch)) {
                return;
            }
            JsonObject basic = snapshotService.basicPlayer(player);
            basic.addProperty("online", false);
            send(epoch, BridgeMessageType.PLAYER_DELTA, null, new BridgePayloads.PlayerDelta("quit", basic));
        }, () -> sendOfflineDelta(epoch, playerId, name));
    }

    /** Called from the network executor after the current socket-bound challenge is checked. */
    private void onHandshakeChallenge(BridgeWebSocketClient.HandshakeChallenge challenge) {
        if (tasks == null || stopping.get()) {
            return;
        }
        tasks.runGlobal(() -> {
            if (!isAwaitingHello(challenge.epoch())) {
                return;
            }
            bridgeClient.sendHello(challenge.epoch(), capabilities());
        });
    }

    /** Called from the network executor only after signed hello_ack verification succeeds. */
    private void onBridgeReady(long epoch) {
        if (tasks == null || stopping.get()) {
            return;
        }
        tasks.runGlobal(() -> {
            if (isReady(epoch)) {
                sendPresenceSync("hello_ack", epoch);
            }
        });
    }

    /** Network-only state is already cleared by the client; no Bukkit access is needed here. */
    private void onBridgeDisconnected(long epoch) {
        // Scheduled Bukkit work is all epoch-bound and will self-discard after this connection ends.
    }

    /**
     * Called by BridgeWebSocketClient's I/O executor after session ID, HMAC, timestamp, and
     * sequence checks. It must not access player state directly.
     */
    private void handleInbound(long epoch, BridgeEnvelope envelope) {
        if (!isReady(epoch)) {
            return;
        }
        switch (envelope.type()) {
            case SNAPSHOT_REQUEST -> handleSnapshotRequest(epoch, envelope);
            case PAPI_REQUEST -> handlePapiRequest(epoch, envelope);
            case ERROR -> getLogger().warning("Backend reported bridge error: " + safeErrorCode(envelope));
            default -> sendError(
                epoch,
                envelope.requestId(),
                "unsupported_message",
                "Inbound " + envelope.type().wireName() + " is not supported by the read-only bridge"
            );
        }
    }

    private void handleSnapshotRequest(long epoch, BridgeEnvelope envelope) {
        if (envelope.requestId() == null || envelope.requestId().isBlank()) {
            sendError(epoch, null, "missing_request_id", "snapshot_request requires request_id");
            return;
        }
        BridgePayloads.SnapshotRequest request;
        UUID playerId;
        try {
            request = protocolCodec.parsePayload(envelope, BridgePayloads.SnapshotRequest.class);
            playerId = UUID.fromString(request.playerUuid());
            if (!validSnapshotSections(request.sections())) {
                throw new IllegalArgumentException("invalid sections");
            }
        } catch (IllegalArgumentException exception) {
            sendError(epoch, envelope.requestId(), "invalid_snapshot_request", "snapshot_request has an invalid player_uuid or sections");
            return;
        }
        String canonicalPlayerId = playerId.toString();
        tasks.runForOnlinePlayer(playerId, player -> {
            if (!isReady(epoch)) {
                return;
            }
            JsonObject snapshot = snapshotService.snapshot(player, request.sections());
            BridgePayloads.SnapshotResponse response = new BridgePayloads.SnapshotResponse(
                "ok",
                canonicalPlayerId,
                snapshot,
                null
            );
            if (!fitsSnapshotResponse(response)) {
                send(
                    epoch,
                    BridgeMessageType.SNAPSHOT_RESPONSE,
                    envelope.requestId(),
                    new BridgePayloads.SnapshotResponse(
                        "unavailable",
                        canonicalPlayerId,
                        new JsonObject(),
                        "snapshot_too_large"
                    )
                );
                return;
            }
            send(
                epoch,
                BridgeMessageType.SNAPSHOT_RESPONSE,
                envelope.requestId(),
                response
            );
        }, () -> {
            if (isReady(epoch)) {
                send(
                    epoch,
                    BridgeMessageType.SNAPSHOT_RESPONSE,
                    envelope.requestId(),
                    new BridgePayloads.SnapshotResponse("unavailable", canonicalPlayerId, new JsonObject(), "player_unavailable")
                );
            }
        });
    }

    private void handlePapiRequest(long epoch, BridgeEnvelope envelope) {
        if (envelope.requestId() == null || envelope.requestId().isBlank()) {
            sendError(epoch, null, "missing_request_id", "papi_request requires request_id");
            return;
        }
        BridgePayloads.PapiRequest request;
        UUID playerId;
        try {
            request = protocolCodec.parsePayload(envelope, BridgePayloads.PapiRequest.class);
            playerId = UUID.fromString(request.playerUuid());
            if (!validPapiFields(request.fields())) {
                throw new IllegalArgumentException("invalid fields");
            }
        } catch (IllegalArgumentException exception) {
            sendError(epoch, envelope.requestId(), "invalid_papi_request", "papi_request has an invalid player_uuid or fields list");
            return;
        }
        String canonicalPlayerId = playerId.toString();
        tasks.runForOnlinePlayer(
            playerId,
            player -> sendPapiResponse(epoch, envelope.requestId(), canonicalPlayerId, player, request.fields()),
            () -> {
                if (isReady(epoch)) {
                    send(
                        epoch,
                        BridgeMessageType.PAPI_RESPONSE,
                        envelope.requestId(),
                        new BridgePayloads.PapiResponse("unavailable", canonicalPlayerId, new JsonObject(), "player_unavailable")
                    );
                }
            }
        );
    }

    /** This method is only invoked from the target player's entity scheduler. */
    private void sendPapiResponse(
        long epoch,
        String requestId,
        String playerId,
        Player player,
        List<BridgePayloads.PapiRequestField> requestedFields
    ) {
        if (!isReady(epoch)) {
            return;
        }
        JsonObject fields = new JsonObject();
        if (!papiResolver.isAvailable()) {
            for (BridgePayloads.PapiRequestField requestedField : requestedFields) {
                JsonObject field = new JsonObject();
                field.addProperty("status", "unavailable");
                field.addProperty("error_code", "papi_unavailable");
                fields.add(requestedField.id(), field);
            }
            send(
                epoch,
                BridgeMessageType.PAPI_RESPONSE,
                requestId,
                new BridgePayloads.PapiResponse("unavailable", playerId, fields, "papi_unavailable")
            );
            return;
        }
        for (BridgePayloads.PapiRequestField requestedField : requestedFields) {
            String fieldId = requestedField.id();
            JsonObject field = new JsonObject();
            String requestedPlaceholder = requestedField.placeholder();
            if (!bridgeConfig.papi().fields().containsValue(requestedPlaceholder)) {
                field.addProperty("status", "denied");
            } else {
                try {
                    field.addProperty("status", "ok");
                    field.addProperty("value", truncate(papiResolver.resolve(player, requestedPlaceholder)));
                } catch (RuntimeException exception) {
                    field.addProperty("status", "error");
                    field.addProperty("error_code", "placeholder_error");
                    getLogger().warning("PAPI field " + fieldId + " failed: " + exception.getClass().getSimpleName());
                }
            }
            fields.add(fieldId, field);
        }
        send(epoch, BridgeMessageType.PAPI_RESPONSE, requestId, new BridgePayloads.PapiResponse("ok", playerId, fields, null));
    }

    /** Must run in the global region. It never reads mutable player state there. */
    private void sendPresenceSync(String reason, long epoch) {
        if (!isReady(epoch)) {
            return;
        }
        Collection<? extends Player> onlinePlayers = Bukkit.getOnlinePlayers();
        if (onlinePlayers.isEmpty()) {
            send(epoch, BridgeMessageType.PRESENCE_SYNC, null, new BridgePayloads.PresenceSync(reason, List.of(), true));
            return;
        }
        ConcurrentLinkedQueue<JsonObject> records = new ConcurrentLinkedQueue<>();
        AtomicInteger remaining = new AtomicInteger(onlinePlayers.size());
        for (Player player : onlinePlayers) {
            tasks.runForPlayer(player, current -> {
                records.add(snapshotService.basicPlayer(current));
                finishPresenceSync(reason, records, remaining, epoch);
            }, () -> finishPresenceSync(reason, records, remaining, epoch));
        }
    }

    private void finishPresenceSync(
        String reason,
        ConcurrentLinkedQueue<JsonObject> records,
        AtomicInteger remaining,
        long epoch
    ) {
        if (remaining.decrementAndGet() != 0 || !isReady(epoch)) {
            return;
        }
        List<JsonObject> sorted = new ArrayList<>(records);
        sorted.sort(Comparator.comparing(record -> record.get("uuid").getAsString()));
        for (int start = 0; start < sorted.size(); start += MAX_PRESENCE_BATCH) {
            int end = Math.min(start + MAX_PRESENCE_BATCH, sorted.size());
            send(
                epoch,
                BridgeMessageType.PRESENCE_SYNC,
                null,
                new BridgePayloads.PresenceSync(reason, List.copyOf(sorted.subList(start, end)), end == sorted.size())
            );
        }
    }

    /** Must run in the global region. */
    private void sendHeartbeat() {
        long epoch = readyEpoch();
        if (!isReady(epoch)) {
            return;
        }
        send(
            epoch,
            BridgeMessageType.HEARTBEAT,
            null,
            new BridgePayloads.Heartbeat(System.currentTimeMillis() - startedAt, Bukkit.getOnlinePlayers().size(), true)
        );
    }

    private void sendOfflineDelta(long epoch, String playerId, String name) {
        if (!isReady(epoch)) {
            return;
        }
        JsonObject basic = new JsonObject();
        basic.addProperty("uuid", playerId);
        basic.addProperty("name", name);
        basic.addProperty("online", false);
        basic.addProperty("observed_at", System.currentTimeMillis());
        send(epoch, BridgeMessageType.PLAYER_DELTA, null, new BridgePayloads.PlayerDelta("quit", basic));
    }

    private List<String> capabilities() {
        List<String> capabilities = new ArrayList<>(List.of(
            "presence",
            "snapshot",
            "inventory_preview",
            "ender_chest",
            "folia_entity_scheduler"
        ));
        if (papiResolver != null && papiResolver.isAvailable()) {
            capabilities.add("papi_read");
        }
        return capabilities;
    }

    private long readyEpoch() {
        return bridgeClient == null ? 0L : bridgeClient.readyEpoch();
    }

    private boolean isAwaitingHello(long epoch) {
        return !stopping.get() && bridgeClient != null && bridgeClient.isAwaitingHello(epoch);
    }

    private boolean isReady(long epoch) {
        return !stopping.get() && bridgeClient != null && bridgeClient.isReady(epoch);
    }

    private void send(long epoch, BridgeMessageType type, String requestId, Object payload) {
        BridgeWebSocketClient client = bridgeClient;
        if (client != null) {
            client.send(epoch, type, requestId, payload);
        }
    }

    private static boolean validSnapshotSections(List<String> sections) {
        if (sections == null || sections.isEmpty() || sections.size() > MAX_SNAPSHOT_SECTIONS) {
            return false;
        }
        Set<String> requested = new HashSet<>();
        for (String section : sections) {
            if (section == null || !ALLOWED_SNAPSHOT_SECTIONS.contains(section) || !requested.add(section)) {
                return false;
            }
        }
        return true;
    }

    private static boolean validPapiFields(List<BridgePayloads.PapiRequestField> fields) {
        if (fields == null || fields.isEmpty() || fields.size() > MAX_PAPI_REQUEST_FIELDS) {
            return false;
        }
        Set<String> ids = new HashSet<>();
        for (BridgePayloads.PapiRequestField field : fields) {
            if (field == null
                || field.id() == null
                || field.id().isBlank()
                || field.id().length() > 64
                || field.placeholder() == null
                || field.placeholder().isBlank()
                || Utf8TextLimiter.byteLength(field.placeholder()) > MAX_PAPI_PLACEHOLDER_LENGTH
                || !ids.add(field.id())) {
                return false;
            }
        }
        return true;
    }

    private void sendError(long epoch, String requestId, String code, String message) {
        send(epoch, BridgeMessageType.ERROR, requestId, new BridgePayloads.Error(code, message));
    }

    private boolean fitsSnapshotResponse(BridgePayloads.SnapshotResponse response) {
        try {
            return protocolCodec.payloadBytesOf(protocolCodec.payloadOf(response)).length
                <= MAX_SNAPSHOT_RESPONSE_PAYLOAD_BYTES;
        } catch (RuntimeException exception) {
            getLogger().warning("Unable to encode a snapshot response: " + exception.getClass().getSimpleName());
            return false;
        }
    }

    private String truncate(String value) {
        return Utf8TextLimiter.truncate(value == null ? "" : value, bridgeConfig.snapshot().maxTextLength());
    }

    private static String safeErrorCode(BridgeEnvelope envelope) {
        try {
            JsonObject payload = envelope.payload();
            return payload.has("code") ? payload.get("code").getAsString() : "unknown";
        } catch (RuntimeException exception) {
            return "malformed";
        }
    }
}
