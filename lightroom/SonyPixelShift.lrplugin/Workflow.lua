local LrApplication = import 'LrApplication'
local LrDialogs = import 'LrDialogs'
local LrFileUtils = import 'LrFileUtils'
local LrFunctionContext = import 'LrFunctionContext'
local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'
local LrPrefs = import 'LrPrefs'
local LrView = import 'LrView'
local Protocol = require 'Protocol'
local Engine = require 'Engine'
local Labels = require 'Labels'
local M = {}
local function write(path, text)
    local file, problem = io.open(path, 'wb')
    assert(file, 'Cannot create ' .. path .. ': ' .. tostring(problem))
    local ok, message = file:write(text)
    local closed, closeProblem = file:close()
    assert(ok and closed, message or closeProblem or 'Cannot write job file')
end
function M.start(denoise)
    LrTasks.startAsyncTask(function()
        local jobPath, resultPath, infoPath, logPath
        local succeeded = false
        local ok, err = LrTasks.pcall(function()
            LrFunctionContext.callWithContext('Sony Pixel Shift', function(context)
                local catalog = LrApplication.activeCatalog()
                local photos = catalog:getTargetPhotos()
                local count = #photos
                assert((denoise and count >= 2 and count <= 64) or
                    (not denoise and (count == 4 or count == 16)),
                    denoise and 'Select 2–64 RAW photos of the same scene.' or 'Select 4 or 16 RAW photos taken in camera Pixel Shift mode.')
                local paths = {}
                for i, photo in ipairs(photos) do
                    paths[i] = photo:getRawMetadata('path')
                    assert(paths[i] and paths[i]:lower():match('%.arw$'), 'Select original ARW files.')
                    assert(LrFileUtils.exists(paths[i]) == 'file', 'An original RAW is offline: ' .. paths[i])
                end
                local preferences = LrPrefs.prefsForPlugin()
                local executable, custom = Engine.executable(_PLUGIN.path, WIN_ENV, preferences,
                    LrFileUtils.exists, LrPathUtils.child)
                local manifest
                if not custom then
                    manifest = Engine.readJson(LrPathUtils.child(_PLUGIN.path, 'manifest.json'))
                    Engine.checkManifest(manifest)
                end
                local summary = Labels.summary(denoise, count)
                local view = LrView.osFactory()
                local choice = LrDialogs.presentModalDialog {
                    title = summary.title,
                    contents = view:column {
                        spacing = view:control_spacing(),
                        view:static_text { title = 'Input: ' .. summary.input },
                        view:static_text { title = 'Result: ' .. summary.result },
                        view:static_text { title = 'Best for: ' .. summary.capture },
                        view:static_text { title = 'Output: 16-bit TIFF' },
                        view:static_text { title = 'Camera: Auto-detected | Experimental' },
                    },
                    actionVerb = 'Choose output', cancelVerb = 'Cancel',
                }
                if choice ~= 'ok' then return end
                local output = LrDialogs.runSavePanel { title = summary.title .. ' | Save TIFF',
                    requiredFileType = 'tif', initialDirectory = LrPathUtils.parent(paths[1]),
                    initialFileName = denoise and 'Sony-Denoised.tif' or 'Sony-PixelShift.tif' }
                if not output then return end
                assert(not LrFileUtils.exists(output), 'Choose a new output filename.')
                jobPath = LrFileUtils.chooseUniqueFileName(output .. '.pixelshift-job.json')
                resultPath = jobPath .. '.result.json'
                infoPath = jobPath .. '.engine.json'
                logPath = LrFileUtils.chooseUniqueFileName(output .. '.pixelshift.log')
                local mode = denoise and 'denoise' or 'auto'
                write(jobPath, Protocol.job(paths, output, mode))
                write(logPath, 'Sony Pixel Shift Lightroom plugin ' .. Engine.version .. '\nBuild: ' .. (manifest and manifest.source_revision or 'development override') .. '\nHelper: ' .. executable .. '\n')
                local status = LrTasks.execute(Engine.command(executable, { 'info', '--output', infoPath }, WIN_ENV))
                if status ~= 0 then
                    local message = Engine.launchError(status, WIN_ENV)
                    if LrFileUtils.exists(infoPath) == 'file' then
                        local readOk, failure = LrTasks.pcall(Engine.readJson, infoPath)
                        if readOk and type(failure) == 'table' and type(failure.message) == 'string' then
                            message = failure.message
                        end
                    end
                    error(message, 0)
                end
                Engine.checkInfo(Engine.readJson(infoPath), manifest)
                status = LrTasks.execute(Engine.command(executable,
                    { 'merge', '--job', jobPath, '--result', resultPath, '--log', logPath }, WIN_ENV))
                local result
                if LrFileUtils.exists(resultPath) == 'file' then result = Engine.readJson(resultPath) end
                assert(status == 0 and result and result.version == 1 and result.status == 'complete',
                    (result and result.message) or ('Engine failed (exit ' .. tostring(status) .. ').'))
                assert(result.output == output, 'The engine reported a different output path; import was stopped.')
                assert(LrFileUtils.exists(output) == 'file', 'Engine did not produce the TIFF.')
                local imported = false
                catalog:withWriteAccessDo('Import Sony Pixel Shift TIFF', function()
                    assert(catalog:addPhoto(output), 'Lightroom did not import the TIFF.')
                    imported = true
                end, { timeout = 30 })
                assert(imported, 'The catalog was busy. The completed TIFF was saved and can be imported manually.')
                succeeded = true
                LrDialogs.message(summary.title, 'Result: ' .. summary.result .. '\nSaved: 16-bit TIFF\nAdded to: Lightroom\nStatus: Experimental\n\nDiagnostic log: ' .. logPath, 'info')
            end)
        end)
        if infoPath then LrFileUtils.delete(infoPath) end
        if succeeded then
            if jobPath then LrFileUtils.delete(jobPath) end
            if resultPath then LrFileUtils.delete(resultPath) end
        end
        if not ok then
            if logPath then
                local file = io.open(logPath, 'ab')
                if file then file:write('\nLightroom workflow: ' .. tostring(err) .. '\n'); file:close() end
            end
            local details = Engine.userError(err)
            if logPath then details = details .. '\n\nDiagnostic log: ' .. logPath end
            if jobPath then details = details .. '\nJob retained for troubleshooting: ' .. jobPath end
            LrDialogs.message('Sony Pixel Shift', details, 'critical')
        end
    end)
end
return M
