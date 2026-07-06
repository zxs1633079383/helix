# Business command ownership identified

The user correctly answered that a new IM command such as `im_mark_channel_read` primarily belongs in `helix-im`, because the business command, outbound request shape, and projection event are module responsibilities rather than core or driver responsibilities. Future lessons can now build on this by teaching how `helix-im` expresses side effects as `Effect` and how `helix-driver-host` dispatches them.
