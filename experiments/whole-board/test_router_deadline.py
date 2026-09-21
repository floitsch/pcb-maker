# Copyright (C) 2026 Toit contributors.
import unittest
from compare_native_router import router_deadline_reached


class RouterDeadlineTest(unittest.TestCase):
    def test_worker_completion_does_not_hide_requested_deadline(self):
        self.assertTrue(router_deadline_reached('JOB_STATE=COMPLETED\nROUTER_DEADLINE_REACHED=true\n'))
        self.assertFalse(router_deadline_reached('JOB_STATE=COMPLETED\nROUTER_DEADLINE_REACHED=false\n'))

    def test_absent_ambiguous_or_embedded_marker_is_not_a_result(self):
        for log in ['', 'JOB_STATE=COMPLETED\n', 'prefix ROUTER_DEADLINE_REACHED=true\n',
                    'ROUTER_DEADLINE_REACHED=true\nROUTER_DEADLINE_REACHED=false\n']:
            with self.assertRaises(ValueError):
                router_deadline_reached(log)


if __name__ == '__main__': unittest.main()
