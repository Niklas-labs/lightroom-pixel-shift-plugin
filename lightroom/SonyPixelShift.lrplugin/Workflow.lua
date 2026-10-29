local LrApplication = import 'LrApplication'
local LrDialogs = import 'LrDialogs'
local LrFileUtils = import 'LrFileUtils'
local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'
local LrPrefs = import 'LrPrefs'
local Protocol = require 'Protocol'
local M = {}
function M.start(denoise)
    LrTasks.startAsyncTask(function()
        local jobPath
        local ok, err = LrTasks.pcall(function()
            local catalog = LrApplication.activeCatalog()
            local photos = catalog:getTargetPhotos()
            local count = #photos
            assert((denoise and count >= 2 and count <= 64) or
                (not denoise and (count == 4 or count == 16)),
                'Select exactly 4 or 16 Pixel Shift ARWs, or 2–64 RAWs for denoise.')
            local paths = {}
            for i, photo in ipairs(photos) do
                paths[i] = photo:getRawMetadata('path')
                assert(paths[i] and paths[i]:lower():match('%.arw$'), 'Select original ARW files.')
                assert(LrFileUtils.exists(paths[i]) == 'file', 'An original RAW is offline.')
            end
            local prefs = LrPrefs.prefsForPlugin()
            local executable = prefs.enginePath
            if not executable or LrFileUtils.exists(executable) ~= 'file' then
                local chosen = LrDialogs.runOpenPanel { title = 'Choose sony-pixelshift executable',
                    canChooseFiles = true, canChooseDirectories = false, allowsMultipleSelection = false }
                if not chosen then return end
                executable = chosen[1]
                prefs.enginePath = executable
            end
            local output = LrDialogs.runSavePanel { title = 'Save merged 16-bit TIFF',
                requiredFileType = 'tif', initialDirectory = LrPathUtils.parent(paths[1]),
                initialFileName = denoise and 'Sony-Denoised.tif' or 'Sony-PixelShift.tif' }
            if not output then return end
            assert(not LrFileUtils.exists(output), 'Choose a new output filename.')
            jobPath = LrFileUtils.chooseUniqueFileName(LrPathUtils.child(
                LrPathUtils.getStandardFilePath('temp'), 'sony-pixelshift-job.json'))
            local file = assert(io.open(jobPath, 'wb'))
            local mode = denoise and 'denoise' or (count == 4 and 'four' or 'sixteen')
            assert(file:write(Protocol.job(paths, output, mode)))
            assert(file:close())
            local command = Protocol.quote(executable, WIN_ENV) .. ' merge --job ' .. Protocol.quote(jobPath, WIN_ENV)
            if WIN_ENV then command = '"' .. command .. '"' end
            local status = LrTasks.execute(command)
            assert(status == 0, 'Engine failed (exit ' .. tostring(status) .. '). Check shot sequence, ExifTool, and available memory. Run the saved inputs from the CLI for detailed diagnostics.')
            assert(LrFileUtils.exists(output) == 'file', 'Engine did not produce the TIFF.')
            catalog:withWriteAccessDo('Import Sony Pixel Shift TIFF', function()
                catalog:addPhoto(output)
            end)
            LrDialogs.message('Sony Pixel Shift', 'TIFF created and imported. Camera profile and motion correction are experimental.', 'info')
        end)
        if jobPath then LrFileUtils.delete(jobPath) end
        if not ok then LrDialogs.message('Sony Pixel Shift', tostring(err), 'critical') end
    end)
end
return M
