package com.sculkcatalyst.paperbridge.papi;

import org.bukkit.plugin.Plugin;

public final class PapiResolvers {
    private PapiResolvers() {
    }

    public static PapiResolver create(Plugin plugin, boolean enabled) {
        if (!enabled) {
            return new UnavailablePapiResolver("PAPI is disabled by bridge configuration");
        }
        if (!plugin.getServer().getPluginManager().isPluginEnabled("PlaceholderAPI")) {
            return new UnavailablePapiResolver("PlaceholderAPI is not installed or enabled");
        }
        try {
            return new PlaceholderApiResolver();
        } catch (LinkageError error) {
            return new UnavailablePapiResolver("PlaceholderAPI API linkage failed");
        }
    }
}
