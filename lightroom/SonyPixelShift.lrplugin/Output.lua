-- Deterministic, non-overwriting names beside one source capture.
local M = {}
function M.path(paths, denoise, pathUtils, chooseUnique)
    assert(#paths > 0, 'Select source RAW photos.')
    local folder = pathUtils.parent(paths[1])
    local names = {}
    for _, path in ipairs(paths) do
        assert(pathUtils.parent(path) == folder, 'Select source photos from one folder.')
        names[#names + 1] = pathUtils.leafName(path):gsub('%.[^%.]+$', '')
    end
    table.sort(names, function(a,b)
        if a:lower() == b:lower() then return a < b end
        return a:lower() < b:lower()
    end)
    local range = names[1]
    if names[#names] ~= range then range = range .. '-' .. names[#names] end
    local suffix = denoise and '-NoiseReduced.dng' or '-PixelShift.dng'
    return chooseUnique(pathUtils.child(folder, range .. suffix))
end
return M
