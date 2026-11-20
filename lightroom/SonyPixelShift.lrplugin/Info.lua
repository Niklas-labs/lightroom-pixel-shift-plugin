-- Commands belong under Library > Plug-in Extras.
local menuItems = {
    { title = 'Pixel Shift: more detail (4 or 16 RAWs)', file = 'Merge.lua' },
    { title = 'Reduce noise: ordinary RAW burst', file = 'Denoise.lua' },
    { title = 'Find Pixel Shift sequences', file = 'FindSequences.lua' },
    { title = 'Next Pixel Shift sequence', file = 'NextSequence.lua' },
    { title = 'Flag entire stack', file = 'FlagStack.lua' },
    { title = 'Reject entire stack', file = 'RejectStack.lua' },
    { title = 'Unflag entire stack', file = 'UnflagStack.lua' },
    { title = 'Sync rejection from stack top', file = 'SyncStackRejection.lua' },
    { title = 'Reject hidden photos in collapsed stacks', file = 'RejectHiddenStackPhotos.lua' },
}
return {
    LrSdkVersion = 6.0,
    LrSdkMinimumVersion = 6.0,
    LrToolkitIdentifier = 'org.sonypixelshift.lightroom',
    LrPluginName = 'Sony Pixel Shift (Experimental)',
    LrLibraryMenuItems = menuItems,
    VERSION = { major = 0, minor = 1, revision = 0, build = 18 },
}
