package com.sculkcatalyst.paperbridge.protocol;

import java.util.Arrays;
import java.util.Optional;

public enum BridgeMessageType {
    HELLO_INIT("hello_init"),
    CHALLENGE("challenge"),
    HELLO("hello"),
    HELLO_ACK("hello_ack"),
    PRESENCE_SYNC("presence_sync"),
    PLAYER_DELTA("player_delta"),
    SNAPSHOT_REQUEST("snapshot_request"),
    SNAPSHOT_RESPONSE("snapshot_response"),
    PAPI_REQUEST("papi_request"),
    PAPI_RESPONSE("papi_response"),
    HEARTBEAT("heartbeat"),
    ERROR("error"),
    BYE("bye");

    private final String wireName;

    BridgeMessageType(String wireName) {
        this.wireName = wireName;
    }

    public String wireName() {
        return wireName;
    }

    public static Optional<BridgeMessageType> fromWireName(String value) {
        return Arrays.stream(values())
            .filter(type -> type.wireName.equals(value))
            .findFirst();
    }
}
