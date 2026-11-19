-- Header-only sequence discovery. Existing-photo stacking is a native Library action.
local LrApplication = import 'LrApplication'
local LrDialogs = import 'LrDialogs'
local LrFileUtils = import 'LrFileUtils'
local LrPathUtils = import 'LrPathUtils'
local LrTasks = import 'LrTasks'
local LrPrefs = import 'LrPrefs'
local LrFunctionContext = import 'LrFunctionContext'
local LrProgressScope = import 'LrProgressScope'
local LrView = import 'LrView'
local LrBinding = import 'LrBinding'
local Engine = require 'Engine'
local Protocol = require 'Protocol'
local M = {}
local busy, saved = false, nil
local function folders(catalog)
    local sources = catalog:getActiveSources()
    assert(type(sources) == 'table' and #sources > 0, 'Open a folder in Library first.')
    for _, source in ipairs(sources) do
        assert(type(source) ~= 'string' and source:type() == 'LrFolder',
            'Open folders in Library first. Collection stacks are not supported.')
    end
end
function M.inputs(catalog)
    folders(catalog)
    assert(catalog:getTargetPhoto(), 'Select photos to search first.')
    local paths, photos, skipped = {}, {}, 0
    for _, photo in ipairs(catalog:getTargetPhotos()) do
        local path = photo:getRawMetadata('path')
        if type(path) == 'string' and path:lower():match('%.arw$') and
            not photo:getRawMetadata('isVirtualCopy') and not photo:getRawMetadata('isInStackInFolder') and not photos[path] then
            paths[#paths+1] = path; photos[path] = photo
        else skipped = skipped+1 end
    end
    assert(#paths > 0, 'Select unstacked original ARWs. Existing stacks, virtual copies and other formats are skipped.')
    assert(#paths <= 100000, 'Select at most 100000 original ARWs.')
    return paths, photos, skipped
end
function M.review(report, photos)
    assert(report.version == 1 and report.status == 'detected' and type(report.groups) == 'table'
        and type(report.ignored) == 'table' and type(report.errors) == 'table', 'Invalid sequence detection result.')
    local complete, lines, used = {}, {}, {}
    for _, group in ipairs(report.groups) do
        assert(type(group.files) == 'table' and type(group.reason) == 'string', 'Invalid sequence group.')
        if group.status == 'complete' then
            assert((group.shot_count == 4 or group.shot_count == 16) and #group.files == group.shot_count,
                'Invalid complete sequence size.')
            for _, path in ipairs(group.files) do
                assert(photos[path] and not used[path], 'Sequence result contains an unknown or duplicate source.')
                used[path] = true
            end
            complete[#complete+1] = group
        end
        if #lines < 100 then
            local warning = type(group.identity_warning) == 'string' and (' | ' .. group.identity_warning) or ''
            lines[#lines+1] = tostring(group.status) .. ': ' .. tostring(#group.files) .. '/' ..
                tostring(group.shot_count) .. ' | ' .. tostring(group.files[1]) .. ' | ' .. group.reason .. warning
        end
    end
    if #report.groups > 100 then lines[#lines+1] = 'More groups omitted from this summary; all complete groups remain selectable.' end
    for index, problem in ipairs(report.errors) do
        if index <= 20 then lines[#lines+1] = 'Unreadable: ' .. tostring(problem.path) .. ' | ' .. tostring(problem.reason) end
    end
    return complete, table.concat(lines, '\n')
end
function M.select(catalog, groups, photos, index)
    folders(catalog)
    local group = assert(groups[index], 'No more sequences. Select a larger set and find sequences again.')
    local selected, ids = {}, {}
    for _, path in ipairs(group.files) do
        local photo = assert(photos[path], 'Sequence source is no longer available. Search again.')
        assert(photo:getRawMetadata('path') == path and not photo:getRawMetadata('isVirtualCopy') and
            not photo:getRawMetadata('isInStackInFolder'), 'A sequence changed or was stacked. Search again.')
        assert(LrFileUtils.exists(path) == 'file', 'Source is offline: ' .. path)
        selected[#selected+1] = photo; ids[photo.localIdentifier] = true
    end
    local others = {}
    for index=2,#selected do others[#others+1] = selected[index] end
    catalog:setSelectedPhotos(selected[1], others)
    LrTasks.yield()
    local actual = catalog:getTargetPhotos()
    assert(#actual == #selected, 'Could not select the complete sequence. Clear Library filters and return to the searched folders.')
    for _, photo in ipairs(actual) do
        assert(ids[photo.localIdentifier], 'Sequence selection was not confirmed. Search again.')
    end
end
local function write(path, text)
    local file = assert(io.open(path, 'wb'))
    assert(file:write(text)); assert(file:close())
end
function M.start(nextOnly)
    LrTasks.startAsyncTask(function()
        if busy then LrDialogs.showBezel('Sequence search is running.', 1.5); return end
        busy = true
        local files, scope = {}, nil
        local ok, problem = LrTasks.pcall(function()
            local catalog = LrApplication.activeCatalog()
            if nextOnly then
                assert(saved and saved.catalog == catalog, 'Run Find Pixel Shift sequences first.')
                local index = saved.index+1
                -- Skip groups already stacked since the search, but never select a partial group.
                while saved.groups[index] do
                    local stacked = false
                    for _, path in ipairs(saved.groups[index].files) do
                        if saved.photos[path]:getRawMetadata('isInStackInFolder') then stacked = true end
                    end
                    if not stacked then break end
                    index = index+1
                end
                M.select(catalog, saved.groups, saved.photos, index)
                saved.index = index
                LrDialogs.showBezel('Sequence ' .. index .. ' of ' .. #saved.groups .. ' selected', 2)
                return
            end
            local paths, photos, skipped = M.inputs(catalog)
            saved = nil
            LrFunctionContext.callWithContext('Find Pixel Shift sequences', function(context)
                scope = LrProgressScope { title='Find Pixel Shift sequences', functionContext=context }
                scope:setCancelable(false); scope:setIndeterminate()
                scope:setCaption('Reading capture metadata: ' .. #paths .. ' ARWs')
                local executable, custom = Engine.executable(_PLUGIN.path, WIN_ENV, LrPrefs.prefsForPlugin(),
                    LrFileUtils.exists, LrPathUtils.child)
                local manifest
                if not custom then
                    manifest = Engine.readJson(LrPathUtils.child(_PLUGIN.path, 'manifest.json'))
                    Engine.checkManifest(manifest)
                end
                local request = LrFileUtils.chooseUniqueFileName(LrPathUtils.child(LrPathUtils.getStandardFilePath('temp'),
                    'SonyPixelShift-sequences.json'))
                local result, infoPath = request .. '.result.json', request .. '.engine.json'
                files = {request, result, infoPath}
                local inputs = {}
                for _, path in ipairs(paths) do inputs[#inputs+1] = Protocol.jsonString(path) end
                write(request, '{"version":1,"inputs":[' .. table.concat(inputs, ',') .. ']}')
                LrTasks.yield()
                local status = LrTasks.execute(Engine.command(executable, {'info','--output',infoPath}, WIN_ENV))
                assert(status == 0, Engine.launchError(status, WIN_ENV))
                local info = Engine.readJson(infoPath)
                Engine.checkInfo(info, manifest)
                assert(info.sequence_detection == true, 'Install the matching plugin build: this helper lacks sequence detection.')
                status = LrTasks.execute(Engine.command(executable,
                    {'detect-sequences','--request',request,'--output',result}, WIN_ENV))
                local report = LrFileUtils.exists(result) == 'file' and Engine.readJson(result)
                assert(status == 0 and report, report and report.message or 'Sequence search failed.')
                local groups, details = M.review(report, photos)
                scope:done()
                if #groups == 0 then
                    LrDialogs.message('Find Pixel Shift sequences', 'No complete sequences found.\n' ..
                        'Search includes selected, unstacked original ARWs only.\n\n' .. details, 'info')
                    saved = nil; return
                end
                local settings = LrBinding.makePropertyTable(context)
                settings.sequence = 1
                local items = {}
                for index, group in ipairs(groups) do
                    items[#items+1] = {title=tostring(group.shot_count) .. ' shots | ' ..
                        LrPathUtils.leafName(group.files[1]) .. ' – ' .. LrPathUtils.leafName(group.files[#group.files]) ..
                        (type(group.identity_warning) == 'string' and ' | Check camera identity' or ''), value=index}
                end
                local view = LrView.osFactory()
                local choice = LrDialogs.presentModalDialog { title='Find Pixel Shift sequences',
                    contents=view:column {
                        spacing=view:control_spacing(),
                        view:static_text {title=#groups .. ' complete | ' .. (#report.groups-#groups) ..
                            ' incomplete/conflicting/unsupported | ' .. #report.errors .. ' unreadable'},
                        view:static_text {title=(#report.ignored+skipped) .. ' skipped | Selected photos only | No RAW pixel decoding'},
                        view:popup_menu {items=items, value=LrView.bind {key='sequence',bind_to_object=settings}},
                        view:scrolled_view {width=760,height=180,horizontal_scroller=false,
                            view:static_text {title=details,width_in_chars=85,height_in_lines=-1}},
                        view:static_text {title='Select sequence, then Photo > Stacking > Group into Stack.'},
                        view:static_text {title='Next: Library > Plug-in Extras > Next Pixel Shift sequence'},
                    }, actionVerb='Select sequence', cancelVerb='Close' }
                if choice == 'ok' then
                    M.select(catalog, groups, photos, settings.sequence)
                    saved = {catalog=catalog,groups=groups,photos=photos,index=settings.sequence}
                end
            end)
        end)
        if scope and not scope:isDone() then scope:done() end
        for _, path in ipairs(files) do LrFileUtils.delete(path) end
        busy = false
        if not ok then LrDialogs.message('Pixel Shift sequences', Engine.userError(problem), 'critical') end
    end)
end
return M
