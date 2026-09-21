// Copyright (C) 2026 Toit contributors.
import app.freerouting.core.*;
import app.freerouting.management.RoutingJobSchedulerActionThread;
import java.nio.file.*;
import app.freerouting.settings.*;
import app.freerouting.settings.sources.*;

public final class RouteOutsideDsn {
    public static void main(String[] args) {
        try {
            run(args);
        } catch (Throwable error) {
            error.printStackTrace();
            System.exit(1); // The jar may still own a monitor thread after failure.
        }
    }

    private static void run(String[] args) throws Exception {
        Long maximumMillis = args.length > 5 ? Math.multiplyExact(Long.parseLong(args[5]), 1000L) : null;
        if (maximumMillis != null && maximumMillis <= 0)
            throw new IllegalArgumentException("Positive routing limit required");
        app.freerouting.Freerouting.globalSettings = new GlobalSettings();
        var source = Path.of(args[0]);
        var job = new RoutingJob();
        job.setInput(source.toFile());
        var manager = NativeOutlineMode.load(source, job, Boolean.parseBoolean(args[3]));
        job.board = manager.get_routing_board();
        var cli = new String[]{"-de", source.toString(), "-do", args[1], "-mp", args[2], "-mt", "1"};
        var merger = new SettingsMerger(new DefaultSettings(), new JsonFileSettings(),
            new CliSettings(cli), new EnvironmentVariablesSource(),
            new DsnFileSettings(job.input.getData(), job.input.getFilename()));
        job.routerSettings = merger.merge();
        job.routerSettings.applyBoardSpecificOptimizations(job.board);
        Files.writeString(Path.of(args[4]), new com.google.gson.GsonBuilder().setPrettyPrinting().create().toJson(job.routerSettings));
        job.state = RoutingJobState.RUNNING;
        var thread = new RoutingJobSchedulerActionThread(job);
        job.thread = thread;
        var failure = new java.util.concurrent.atomic.AtomicReference<Throwable>();
        thread.setUncaughtExceptionHandler((t, error) -> { failure.set(error); job.finishedAt = java.time.Instant.now(); thread.requestStop(); });
        thread.start();
        boolean deadlineReached = false;
        if (maximumMillis != null) {
            thread.join(maximumMillis);
            if (thread.isAlive()) {
                deadlineReached = true;
                thread.requestStop();
                // Export only after the worker relinquishes its mutable board.
                // The caller also keeps a hard process/pipeline watchdog.
                thread.join(30_000L);
                if (thread.isAlive()) throw new IllegalStateException("Router did not stop safely within grace period");
            }
        } else {
            thread.join();
        }
        if (failure.get() != null) throw new IllegalStateException("Worker failed", failure.get());
        if (job.state != RoutingJobState.COMPLETED && !(deadlineReached && job.state == RoutingJobState.TIMED_OUT)) throw new IllegalStateException("Job state: " + job.state);
        if (job.board.get_outline().keepout_outside_outline_generated() != Boolean.parseBoolean(args[3]))
            throw new IllegalStateException("Routing lost outline mode");
        manager.replaceRoutingBoard(job.board);
        try (var output = Files.newOutputStream(Path.of(args[1]))) {
            if (!manager.saveAsSpecctraSessionSes(output, source.getFileName().toString()))
                throw new IllegalStateException("Session export failed");
        }
        System.out.println("OUTSIDE_MODE=" + job.board.get_outline().keepout_outside_outline_generated());
        System.out.println("JOB_STATE=" + job.state);
        System.out.println("ROUTER_DEADLINE_REACHED=" + deadlineReached);
        System.exit(0); // Match the pinned CLI shutdown after successful export.
    }
}
