-- Sequence search never writes catalog metadata or invokes keyboard automation.
local function scenario(name)
    local state = {files={},errors={},bezel={},selected={},deleted={},commands={},scopes={}}
    local photos, paths = {}, {}
    for i=1,8 do
        local p = {localIdentifier=i, path='/photos/DSC' .. i .. '.ARW'}
        function p:getRawMetadata(key)
            return ({path=self.path,isInStackInFolder=self.stacked,isVirtualCopy=self.virtual})[key]
        end
        photos[i]=p; paths[p.path]=p; state.files[p.path]=true
    end
    state.selected=photos
    local folder={type=function() return 'LrFolder' end}
    local catalog={}
    function catalog:getActiveSources() return name=='collection' and {{type=function() return 'LrCollection' end}} or {folder} end
    function catalog:getTargetPhoto() return name~='empty' and state.selected[1] or nil end
    function catalog:getTargetPhotos() return state.selected end
    function catalog:setSelectedPhotos(top, others)
        state.selected={top}
        if name~='filtered' then for _,p in ipairs(others) do state.selected[#state.selected+1]=p end end
    end
    if name=='existing-stack' then photos[1].stacked=true end
    if name=='virtual' then photos[1].virtual=true end
    if name=='non-arw' then photos[1].path='/photos/other.JPG' end
    local report={version=1,status='detected',groups={},ignored={},errors={}}
    for group=1,2 do
        local files={}
        for i=(group-1)*4+1,group*4 do files[#files+1]=photos[i].path end
        report.groups[#report.groups+1]={status='complete',shot_count=4,files=files,reason='Complete'}
    end
    if name=='incomplete' then report.groups[1].status='incomplete'; report.groups[2].status='incomplete' end
    if name=='unknown-source' then report.groups[1].files[1]='/outside.ARW' end
    if name=='duplicate-source' then report.groups[2].files[1]=report.groups[1].files[1] end
    if name=='bad-count' then report.groups[1].shot_count=16 end
    if name=='wrong-version' then report.version=9 end
    local tasks={startAsyncTask=function(fn) fn() end,pcall=pcall,yield=function() end}
    function tasks.execute(args)
        state.commands[#state.commands+1]=args[1]
        if name=='launch-failure' then return 126 end
        if args[1]=='info' then state.files[args[3]]={sequence_detection=name~='old-helper'}
        else
            assert(args[1]=='detect-sequences')
            assert(state.files[args[3]]:find('"inputs"',1,true))
            state.files[args[5]]=name=='engine-error' and {version=1,status='error',message='Bad request'} or report
            if name=='offline' then state.files[photos[1].path]=nil end
        end
        return name=='engine-error' and args[1]~='info' and 1 or 0
    end
    local dialogs={showBezel=function(text) state.bezel[#state.bezel+1]=text end,
        message=function(title,text,kind) state.errors[#state.errors+1]={text=text,kind=kind} end}
    function dialogs.presentModalDialog(spec)
        assert(spec.actionVerb=='Select sequence')
        state.dialog=spec
        return name=='cancel' and 'cancel' or 'ok'
    end
    local function node(spec) return spec end
    local factory={column=node,static_text=node,popup_menu=node,scrolled_view=node,control_spacing=function() return 4 end}
    local engine={executable=function() return '/helper',true end,command=function(_,args) return args end,
        checkInfo=function(info) assert(type(info)=='table') end,launchError=function() return 'Launch failed' end,
        readJson=function(path) return assert(state.files[path]) end,userError=tostring}
    local sdk={LrApplication={activeCatalog=function() return catalog end},LrTasks=tasks,LrDialogs=dialogs,
        LrFileUtils={exists=function(path) return state.files[path] and 'file' end,
            chooseUniqueFileName=function(p) return p end,delete=function(p) state.deleted[#state.deleted+1]=p;state.files[p]=nil end},
        LrPathUtils={child=function(a,b) return a .. '/' .. b end,getStandardFilePath=function() return '/tmp' end,
            leafName=function(p) return p:match('[^/]+$') end},LrPrefs={prefsForPlugin=function() return {} end},
        LrFunctionContext={callWithContext=function(_,fn) fn({}) end},
        LrView={osFactory=function() return factory end,bind=function(p) return p end},
        LrBinding={makePropertyTable=function() return {} end}}
    sdk.LrProgressScope=function()
        local scope={setCancelable=function(_,v) assert(v==false) end,setIndeterminate=function() end,
            setCaption=function() end,done=function(s) s.ended=true end,isDone=function(s) return s.ended end}
        state.scopes[#state.scopes+1]=scope; return scope
    end
    local env=setmetatable({import=function(key) return assert(sdk[key],key) end,WIN_ENV=false,_PLUGIN={path='/plugin'},
        require=function(key) return key=='Engine' and engine or require(key) end,
        io={open=function(path)
            return {write=function(_,text) state.files[path]=text; return true end,close=function() return true end}
        end}}, {__index=_G})
    local chunk=assert(loadfile('lightroom/SonyPixelShift.lrplugin/Sequences.lua'));setfenv(chunk,env)
    local module=chunk()
    if name=='empty' or name=='collection' then
        module.start(false); assert(#state.commands==0 and #state.errors==1)
    elseif name=='existing-stack' or name=='virtual' or name=='non-arw' then
        local inputs,_,skipped=module.inputs(catalog); assert(#inputs==7 and skipped==1)
    else
        module.start(false)
        for _,scope in ipairs(state.scopes) do assert(scope.ended) end
        assert(#state.deleted==3)
        if name=='normal' or name=='next' then
            assert(#state.errors==0 and #state.selected==4 and state.selected[1]==photos[1])
            if name=='next' then
                for i=1,4 do photos[i].stacked=true end
                module.start(true)
                assert(#state.errors==0 and #state.selected==4 and state.selected[1]==photos[5])
                module.start(true); assert(#state.errors==1) -- exhausted; no wrapping.
            end
        elseif name=='cancel' then assert(#state.errors==0 and #state.selected==8)
        elseif name=='incomplete' then assert(#state.errors==1 and state.errors[1].kind=='info' and #state.selected==8)
        else assert(#state.errors==1 and state.errors[1].kind=='critical',name) end
    end
end
local cases={'normal','next','empty','collection','existing-stack','virtual','non-arw','incomplete',
    'cancel','unknown-source','duplicate-source','bad-count','wrong-version','launch-failure','old-helper','engine-error','offline','filtered'}
for _,name in ipairs(cases) do scenario(name) end
print('Sequence search SDK checks passed (' .. #cases .. ' scenarios)')
