package com.sculkcatalyst.paperbridge.papi;

import me.clip.placeholderapi.PlaceholderAPI;
import org.bukkit.entity.Player;

/** Is loaded only after the server confirms that PlaceholderAPI is enabled. */
public final class PlaceholderApiResolver implements PapiResolver {
    @Override
    public boolean isAvailable() {
        return true;
    }

    @Override
    public String resolve(Player player, String placeholder) {
        return PlaceholderAPI.setPlaceholders(player, placeholder);
    }
}
