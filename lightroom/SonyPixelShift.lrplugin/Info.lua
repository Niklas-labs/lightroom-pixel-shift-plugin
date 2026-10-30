return {
    LrSdkVersion = 6.0,
    LrSdkMinimumVersion = 6.0,
    LrToolkitIdentifier = 'org.sonypixelshift.lightroom',
    LrPluginName = 'Sony Pixel Shift (Experimental)',
    LrLibraryMenuItems = {
        { title = 'Pixel Shift: more detail (4 or 16 RAWs)', file = 'Merge.lua' },
        { title = 'Reduce noise: ordinary RAW burst', file = 'Denoise.lua' },
    },
    VERSION = { major = 0, minor = 1, revision = 0, build = 1 },
}
