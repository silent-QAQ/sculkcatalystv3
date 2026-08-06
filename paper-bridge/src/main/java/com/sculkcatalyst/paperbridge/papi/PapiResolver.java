package com.sculkcatalyst.paperbridge.papi;

import org.bukkit.entity.Player;

/** Must be called from the player's entity scheduler context. */
public interface PapiResolver {
    boolean isAvailable();

    String resolve(Player player, String placeholder);

    default String unavailableReason() {
        return null;
    }
}
