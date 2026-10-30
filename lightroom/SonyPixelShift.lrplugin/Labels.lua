-- Short outcome labels; Pixel Shift modes require an actual camera capture.
local M = {}
function M.summary(denoise, count)
    if denoise then
        return {
            title = 'Reduce noise',
            input = count .. ' ordinary RAW photos',
            result = 'Less noise | Original resolution',
            capture = 'Same scene | Same exposure | Minimal movement',
        }
    end
    return {
        title = 'Merge Pixel Shift',
        input = count .. ' RAW photos taken in camera Pixel Shift mode',
        result = count == 16 and 'Higher resolution | Up to 2x width and height' or 'Better color detail | Original resolution',
        capture = 'Tripod | Still scene',
    }
end
return M
