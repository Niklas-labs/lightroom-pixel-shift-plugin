local LrApplication = import 'LrApplication'
local LrDialogs = import 'LrDialogs'
local LrFileUtils = import 'LrFileUtils'
local LrFunctionContext = import 'LrFunctionContext'
local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'
local LrPrefs = import 'LrPrefs'
local LrView = import 'LrView'
local LrProgressScope = import 'LrProgressScope'
local Protocol = require 'Protocol'
local Engine = require 'Engine'
local Labels = require 'Labels'
local Output = require 'Output'
local Progress = require 'Progress'
local M = {}
local function write(path, text)
    local file, problem = io.open(path, 'wb')
    assert(file, 'Cannot create ' .. path .. ': ' .. tostring(problem))
    local ok, message = file:write(text)
    local closed, closeProblem = file:close()
    assert(ok and closed, message or closeProblem or 'Cannot write job file')
end
local function append(path, text)
    local file = assert(io.open(path, 'ab'))
    assert(file:write(text .. '\n')); assert(file:close())
end
function M.start(denoise)
    LrTasks.startAsyncTask(function()
        local jobPath, resultPath, infoPath, logPath, progress
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
                local output = Output.path(paths, denoise, LrPathUtils, LrFileUtils.chooseUniqueFileName)
                local summary = Labels.summary(denoise, count)
                local view = LrView.osFactory()
                local choice = LrDialogs.presentModalDialog {
                    title = summary.title,
                    contents = view:column {
                        spacing = view:control_spacing(),
                        view:static_text { title = 'Input: ' .. summary.input },
                        view:static_text { title = 'Result: ' .. summary.result },
                        view:static_text { title = 'Best for: ' .. summary.capture },
                        view:static_text { title = 'Output: Linear DNG (uncompressed)' },
                        view:static_text { title = 'Save as: ' .. LrPathUtils.leafName(output) },
                        view:static_text { title = 'Folder: Beside source photos' },
                        view:static_text { title = 'Camera: Auto-detected | Experimental' },
                    },
                    actionVerb = denoise and 'Reduce noise' or 'Merge', cancelVerb = 'Cancel',
                }
                if choice ~= 'ok' then return end
                assert(not LrFileUtils.exists(output), 'Output already exists. Retry to choose a unique name.')
                progress = Progress.new(LrProgressScope { title=summary.title, functionContext=context }, count, LrTasks)
                LrTasks.yield() -- Let Lightroom paint the scope before launching the helper.
                jobPath = LrFileUtils.chooseUniqueFileName(output .. '.pixelshift-job.json')
                resultPath = jobPath .. '.result.json'
                infoPath = jobPath .. '.engine.json'
                logPath = LrFileUtils.chooseUniqueFileName(output .. '.pixelshift.log')
                local mode = denoise and 'denoise' or 'auto'
                write(jobPath, Protocol.job(paths, output, mode))
                write(logPath, 'Sony Pixel Shift Lightroom plugin ' .. Engine.version .. '\nBuild: ' .. (manifest and manifest.source_revision or 'development override') .. '\nHelper: ' .. executable .. '\n')
                progress:stage('Checking engine')
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
                status = progress:run(Engine.command(executable,
                    { 'merge', '--job', jobPath, '--result', resultPath, '--log', logPath }, WIN_ENV), logPath)
                local result
                if LrFileUtils.exists(resultPath) == 'file' then result = Engine.readJson(resultPath) end
                assert(status == 0 and result and result.version == 1 and result.status == 'complete',
                    (result and result.message) or ('Engine failed (exit ' .. tostring(status) .. ').'))
                assert(result.output == output, 'The engine reported a different output path; import was stopped.')
                assert(LrFileUtils.exists(output) == 'file', 'Engine did not produce the DNG.')
                progress:stage('Adding to Lightroom')
                local callbackRan = false
                local existing = catalog:findPhotoByPath(output)
                local writeStatus = catalog:withWriteAccessDo('Import Sony Pixel Shift DNG', function()
                    assert(existing or catalog:addPhoto(output), 'Lightroom did not import the DNG.')
                    callbackRan = true
                end, { timeout = 30 })
                append(logPath, 'Catalog write: ' .. tostring(writeStatus))
                assert(callbackRan, 'Catalog busy. DNG saved; import it manually: ' .. output)
                -- Verify after the write gate commits, independently of library filters.
                local imported
                for attempt = 1,3 do
                    imported = catalog:findPhotoByPath(output)
                    if imported then break end
                    if attempt < 3 then LrTasks.sleep(0.1) end
                end
                assert(imported, 'DNG saved, but catalog import was not confirmed: ' .. output)
                append(logPath, 'Catalog lookup: confirmed | ' .. output)
                local selectOk, selectProblem = LrTasks.pcall(function()
                    catalog:setSelectedPhotos(imported, {})
                    LrTasks.yield()
                end)
                local selected = catalog:getTargetPhoto()
                local visible = selectOk and selected and selected:getRawMetadata('path') == output
                append(logPath, 'Selection: ' .. (visible and 'result selected' or 'not selected; check library filters/current source'))
                if not selectOk then append(logPath, 'Selection detail: ' .. tostring(selectProblem)) end
                succeeded = true
                progress:done()
                local visibility = visible and 'View: Result selected' or 'View: Check library filters or current collection'
                LrDialogs.message(summary.title,
                    'Saved: ' .. LrPathUtils.leafName(output) .. '\nCatalog: Confirmed\n' .. visibility ..
                    '\nFolder: ' .. LrPathUtils.parent(output) .. '\n\nDiagnostic log: ' .. logPath, 'info')
            end)
        end)
        if progress then progress:done() end
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
