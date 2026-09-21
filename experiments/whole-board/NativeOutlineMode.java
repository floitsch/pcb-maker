// Copyright (C) 2026 Toit contributors.
import app.freerouting.board.*;
import app.freerouting.core.RoutingJob;
import app.freerouting.interactive.HeadlessBoardManager;
import java.nio.file.*;

public final class NativeOutlineMode {
    public static HeadlessBoardManager load(Path source, RoutingJob job, boolean outside) throws Exception {
        var manager = new HeadlessBoardManager(job);
        try (var input = Files.newInputStream(source)) {
            var result = manager.loadFromSpecctraDsn(input, new BoardObserverAdaptor(),
                new ItemIdentificationNumberGenerator());
            if (!result.toString().equals("OK")) throw new IllegalStateException(result.toString());
        }
        var board = manager.get_routing_board();
        if (board == null || board.get_outline() == null) throw new IllegalStateException("Missing outline");
        if (outside) {
            var matrix = board.rules.clearance_matrix;
            int edgeClass = matrix.get_no("pcbmakeredge");
            if (edgeClass >= 0 && board.get_outline().clearance_class_no() == edgeClass) {
                // The translator declares only edge-edge values. Referencing
                // native classes in the structure scope would create them
                // before Network initializes mixed clearances and pad/via roles.
                for (int layer = 0; layer < matrix.get_layer_count(); layer++) {
                    int clearance = matrix.get_value(edgeClass, edgeClass, layer, false);
                    for (int other = 1; other < matrix.get_class_count(); other++) {
                        matrix.set_value(edgeClass, other, layer, clearance);
                        matrix.set_value(other, edgeClass, layer, clearance);
                    }
                }
                board.search_tree_manager.clearance_value_changed();
            }
            board.get_outline().generate_keepout_outside(true);
        }
        if (board.get_outline().keepout_outside_outline_generated() != outside)
            throw new IllegalStateException("Outline mode mismatch");
        return manager;
    }
}
