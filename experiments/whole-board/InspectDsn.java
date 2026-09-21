// Copyright (C) 2026 Toit contributors.
// Read-only diagnostic for the pinned Freerouting jar. No routing is performed.
import app.freerouting.board.*;
import app.freerouting.core.RoutingJob;
import app.freerouting.interactive.HeadlessBoardManager;
import app.freerouting.interactive.ClearanceViolations;
import app.freerouting.io.specctra.parser.CoordinateTransform;
import com.google.gson.GsonBuilder;
import java.nio.file.*;
import java.util.*;

public class InspectDsn {
    static Map<String, Object> item(Item item, CoordinateTransform transform) {
        var result = new LinkedHashMap<String, Object>();
        result.put("id", item.get_id_no());
        result.put("kind", item.getClass().getSimpleName());
        result.put("component", item.component_name());
        if (item instanceof Pin pin) result.put("pin", pin.name());
        result.put("dsn_bounds", transform.board_to_dsn(item.bounding_box()));
        var nets = new ArrayList<String>();
        for (int i = 0; i < item.net_count(); i++)
            nets.add(item.board.rules.nets.get(item.get_net_no(i)).name);
        result.put("nets", nets);
        return result;
    }

    public static void main(String[] args) throws Exception {
        boolean outside = args.length > 2 && Boolean.parseBoolean(args[2]);
        var manager = NativeOutlineMode.load(Path.of(args[0]), new RoutingJob(), outside);
        String loaded = "OK";
        var board = manager.get_routing_board();
        if (board == null) throw new IllegalStateException(loaded);
        var transform = board.communication.coordinate_transform;
        var layers = new ArrayList<Map<String, Object>>();
        var routerSettings = manager.getCurrentRoutingJob().routerSettings;
        for (int index = 0; index < board.layer_structure.arr.length; index++) {
            var layer = board.layer_structure.arr[index];
            var row = new LinkedHashMap<String, Object>();
            row.put("index",index);
            row.put("name",layer.name);
            row.put("is_signal",layer.is_signal);
            if (routerSettings != null && routerSettings.isLayerActive != null && index < routerSettings.isLayerActive.length)
                row.put("router_active",routerSettings.get_layer_active(index));
            layers.add(row);
        }
        var matrixRows = new ArrayList<Map<String, Object>>();
        var allMatrix = board.rules.clearance_matrix;
        for (int first = 0; first < allMatrix.get_class_count(); first++) {
            for (int second = 0; second < allMatrix.get_class_count(); second++) {
                var values = new ArrayList<Double>();
                for (int layer = 0; layer < allMatrix.get_layer_count(); layer++)
                    values.add(transform.board_to_dsn(allMatrix.get_value(first,second,layer,false)));
                matrixRows.add(Map.of("first",allMatrix.get_name(first),"second",allMatrix.get_name(second),
                    "clearance_per_layer_dsn",values));
            }
        }
        var netClasses = new TreeMap<String, Object>();
        for (int index = 0; index < board.rules.net_classes.count(); index++) {
            var netClass = board.rules.net_classes.get(index);
            var roles = new TreeMap<String, String>();
            for (var role : app.freerouting.rules.DefaultItemClearanceClasses.ItemClass.values())
                roles.put(role.toString(), allMatrix.get_name(netClass.default_item_clearance_classes.get(role)));
            var widths = new ArrayList<Double>();
            for (int layer = 0; layer < allMatrix.get_layer_count(); layer++)
                widths.add(transform.board_to_dsn(2 * netClass.get_trace_half_width(layer)));
            netClasses.put(netClass.get_name(), Map.of("trace_class", allMatrix.get_name(netClass.get_trace_clearance_class()),
                "item_classes", roles, "trace_width_per_layer_dsn", widths));
        }
        var itemClasses = new ArrayList<String>();
        var gson = new GsonBuilder().create();
        for (var copper : board.get_items()) {
            if (!(copper instanceof Pin || copper instanceof Via || copper instanceof PolylineTrace || copper instanceof ConductionArea)) continue;
            var row = item(copper, transform);
            row.remove("id");
            row.put("clearance_class", allMatrix.get_name(copper.clearance_class_no()));
            itemClasses.add(gson.toJson(row));
        }
        Collections.sort(itemClasses);
        var edges = new ArrayList<Map<String, Object>>();
        for (var item : board.get_items()) {
            if (!(item instanceof BoardOutline outline)) continue;
            var matrix = board.rules.clearance_matrix;
            var row = new LinkedHashMap<String, Object>();
            row.put("outside_area", outline.keepout_outside_outline_generated());
            row.put("half_width_dsn", transform.board_to_dsn(outline.get_half_width()));
            row.put("clearance_class", matrix.get_name(outline.clearance_class_no()));
            var classes = new ArrayList<Map<String, Object>>();
            for (int c = 1; c < matrix.get_class_count(); c++) {
                var values = new ArrayList<Double>();
                for (int layer = 0; layer < matrix.get_layer_count(); layer++)
                    values.add(transform.board_to_dsn(matrix.get_value(outline.clearance_class_no(),c,layer,false)));
                classes.add(Map.of("name",matrix.get_name(c),"clearance_per_layer_dsn",values));
            }
            row.put("classes",classes);
            edges.add(row);
        }
        var violations = new ArrayList<Map<String, Object>>();
        for (var violation : new ClearanceViolations(board.get_items()).list) {
            var row = new LinkedHashMap<String, Object>();
            row.put("first", item(violation.first_item, transform));
            row.put("second", item(violation.second_item, transform));
            row.put("layer", violation.layer);
            row.put("dsn_bounds", transform.board_to_dsn(violation.shape.bounding_box()));
            row.put("expected_clearance_dsn", transform.board_to_dsn(violation.expected_clearance));
            row.put("actual_clearance_dsn", transform.board_to_dsn(violation.actual_clearance));
            violations.add(row);
        }
        var result = new LinkedHashMap<String, Object>();
        result.put("load_result", loaded);
        result.put("unit", board.communication.unit.toString());
        result.put("resolution", board.communication.resolution);
        result.put("item_count", board.get_items().size());
        result.put("routing_layers",layers);
        result.put("board_edges", edges);
        result.put("clearance_matrix",matrixRows);
        result.put("net_classes", netClasses);
        result.put("copper_item_clearance_classes", itemClasses);
        result.put("violations", violations);
        Files.writeString(Path.of(args[1]), new GsonBuilder().setPrettyPrinting().create().toJson(result)+"\n");
    }
}
