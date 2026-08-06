package com.sculkcatalyst.paperbridge.platform;

import io.papermc.paper.threadedregions.scheduler.GlobalRegionScheduler;
import io.papermc.paper.threadedregions.scheduler.ScheduledTask;
import java.util.Objects;
import java.util.UUID;
import java.util.function.Consumer;
import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.plugin.Plugin;

/**
 * The only scheduler gateway used by this plugin. It is valid on Paper and Folia.
 * Global work must not inspect mutable player state; entity work is always routed
 * through {@link Player#getScheduler()}.
 */
public final class FoliaTaskDispatcher {
    private final Plugin plugin;
    private final GlobalRegionScheduler globalScheduler;

    public FoliaTaskDispatcher(Plugin plugin) {
        this.plugin = Objects.requireNonNull(plugin, "plugin");
        this.globalScheduler = Bukkit.getGlobalRegionScheduler();
    }

    public void runGlobal(Runnable task) {
        globalScheduler.execute(plugin, task);
    }

    public ScheduledTask runGlobalAtFixedRate(Consumer<ScheduledTask> task, long initialDelayTicks, long periodTicks) {
        return globalScheduler.runAtFixedRate(plugin, task, initialDelayTicks, periodTicks);
    }

    /**
     * Runs against the entity's owning region. The retired callback must not touch Bukkit state.
     * It may be invoked synchronously by the entity scheduler when scheduling is impossible.
     */
    public boolean runForPlayer(Player player, Consumer<Player> task, Runnable retired) {
        Objects.requireNonNull(player, "player");
        Objects.requireNonNull(task, "task");
        Objects.requireNonNull(retired, "retired");
        ScheduledTask scheduled = player.getScheduler().run(plugin, ignored -> task.accept(player), retired);
        if (scheduled == null) {
            retired.run();
            return false;
        }
        return true;
    }

    /** Resolves online state in the global region, then reads player data on the entity region. */
    public void runForOnlinePlayer(UUID playerId, Consumer<Player> task, Runnable unavailable) {
        Objects.requireNonNull(playerId, "playerId");
        Objects.requireNonNull(task, "task");
        Objects.requireNonNull(unavailable, "unavailable");
        runGlobal(() -> {
            Player player = Bukkit.getPlayer(playerId);
            if (player == null) {
                unavailable.run();
                return;
            }
            runForPlayer(player, current -> {
                if (current.isOnline()) {
                    task.accept(current);
                } else {
                    unavailable.run();
                }
            }, unavailable);
        });
    }

    public void cancelOwnedGlobalTasks() {
        globalScheduler.cancelTasks(plugin);
    }
}
