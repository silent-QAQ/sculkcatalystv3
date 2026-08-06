package com.sculkcatalyst.paperbridge.config;

import com.sculkcatalyst.paperbridge.text.Utf8TextLimiter;
import java.net.URI;
import java.net.URISyntaxException;
import java.time.Duration;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Pattern;
import org.bukkit.configuration.ConfigurationSection;
import org.bukkit.configuration.file.FileConfiguration;

/** Immutable, validated operator configuration. */
public record BridgeConfig(
    boolean enabled,
    String serverId,
    URI backendWsUri,
    String token,
    Network network,
    SnapshotLimits snapshot,
    Papi papi
) {
    private static final Pattern FIELD_ID = Pattern.compile("[A-Za-z0-9._-]{1,64}");
    private static final int MAX_TEXT_BYTES = 256;
    private static final int MAX_LORE_LINES = 12;

    public static BridgeConfig load(FileConfiguration yaml) throws ConfigException {
        boolean enabled = yaml.getBoolean("enabled", false);
        String serverId = string(yaml, "server-id", "");
        String endpoint = string(yaml, "backend-ws-url", "");
        String token = string(yaml, "token", "");

        List<String> errors = new java.util.ArrayList<>();
        URI endpointUri = parseEndpoint(endpoint, errors);
        if (enabled) {
            if (serverId.isBlank() || "change-me".equalsIgnoreCase(serverId)) {
                errors.add("server-id must be configured when enabled is true");
            }
            if (token.isBlank() || "CHANGE_ME".equals(token) || token.length() < 24) {
                errors.add("token must be a non-placeholder value of at least 24 characters when enabled is true");
            }
        }

        Network network = new Network(
            boundedInt(yaml, "network.outbound-queue-capacity", 256, 16, 4_096, errors),
            Duration.ofSeconds(boundedInt(yaml, "network.connect-timeout-seconds", 10, 1, 120, errors)),
            Duration.ofSeconds(boundedInt(yaml, "network.reconnect-max-seconds", 30, 1, 300, errors)),
            boundedInt(yaml, "network.heartbeat-seconds", 15, 5, 300, errors)
        );
        SnapshotLimits snapshot = new SnapshotLimits(
            boundedInt(yaml, "snapshot.max-text-length", 256, 32, MAX_TEXT_BYTES, errors),
            boundedInt(yaml, "snapshot.max-lore-lines", 12, 0, MAX_LORE_LINES, errors),
            boundedInt(yaml, "snapshot.max-preview-depth", 1, 0, 3, errors),
            boundedInt(yaml, "snapshot.max-preview-items", 27, 0, 54, errors)
        );
        Papi papi = new Papi(yaml.getBoolean("papi.enabled", true), readPapiFields(yaml, errors));

        if (!errors.isEmpty()) {
            throw new ConfigException(String.join("; ", errors));
        }
        return new BridgeConfig(enabled, serverId, endpointUri, token, network, snapshot, papi);
    }

    private static String string(FileConfiguration yaml, String path, String defaultValue) {
        String value = yaml.getString(path, defaultValue);
        return value == null ? defaultValue : value.trim();
    }

    private static URI parseEndpoint(String endpoint, List<String> errors) {
        try {
            URI uri = new URI(endpoint);
            String scheme = uri.getScheme() == null ? "" : uri.getScheme().toLowerCase(Locale.ROOT);
            if (!(scheme.equals("ws") || scheme.equals("wss")) || uri.getHost() == null) {
                errors.add("backend-ws-url must be an absolute ws:// or wss:// URI");
            }
            return uri;
        } catch (URISyntaxException exception) {
            errors.add("backend-ws-url is not a valid URI");
            return URI.create("ws://127.0.0.1:1/");
        }
    }

    private static int boundedInt(
        FileConfiguration yaml,
        String path,
        int defaultValue,
        int min,
        int max,
        List<String> errors
    ) {
        int value = yaml.getInt(path, defaultValue);
        if (value < min || value > max) {
            errors.add(path + " must be between " + min + " and " + max);
            return defaultValue;
        }
        return value;
    }

    private static Map<String, String> readPapiFields(FileConfiguration yaml, List<String> errors) {
        ConfigurationSection section = yaml.getConfigurationSection("papi.fields");
        if (section == null) {
            return Map.of();
        }
        Map<String, String> fields = new LinkedHashMap<>();
        for (String id : section.getKeys(false)) {
            String placeholder = section.getString(id);
            if (!FIELD_ID.matcher(id).matches()) {
                errors.add("papi.fields contains an invalid field id: " + id);
                continue;
            }
            if (placeholder == null || placeholder.isBlank() || Utf8TextLimiter.byteLength(placeholder) > MAX_TEXT_BYTES) {
                errors.add("papi.fields." + id + " must be a non-blank placeholder of at most 256 UTF-8 bytes");
                continue;
            }
            fields.put(id, placeholder);
        }
        if (fields.size() > 64) {
            errors.add("papi.fields may contain at most 64 fields");
        }
        return Map.copyOf(fields);
    }

    public record Network(
        int outboundQueueCapacity,
        Duration connectTimeout,
        Duration reconnectMaxDelay,
        int heartbeatSeconds
    ) {
    }

    public record SnapshotLimits(
        int maxTextLength,
        int maxLoreLines,
        int maxPreviewDepth,
        int maxPreviewItems
    ) {
    }

    public record Papi(boolean enabled, Map<String, String> fields) {
    }
}
