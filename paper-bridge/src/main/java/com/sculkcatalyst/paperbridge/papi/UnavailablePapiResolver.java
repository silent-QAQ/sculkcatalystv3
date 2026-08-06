package com.sculkcatalyst.paperbridge.papi;

import org.bukkit.entity.Player;

public final class UnavailablePapiResolver implements PapiResolver {
    private final String reason;

    public UnavailablePapiResolver(String reason) {
        this.reason = reason;
    }

    @Override
    public boolean isAvailable() {
        return false;
    }

    @Override
    public String resolve(Player player, String placeholder) {
        throw new IllegalStateException(reason);
    }

    @Override
    public String unavailableReason() {
        return reason;
    }
}
