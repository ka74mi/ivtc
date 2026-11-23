use ivtc_core::passthrough::{PassError, Passthrough, SampleMeta};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG};
use windows::Win32::Media::DxMediaObjects::{
    DMO_E_NOTACCEPTING, DMO_E_TYPE_NOT_ACCEPTED, DMO_E_TYPE_NOT_SET,
    DMO_INPUT_DATA_BUFFERF_DISCONTINUITY, DMO_INPUT_DATA_BUFFERF_TIME,
    DMO_INPUT_DATA_BUFFERF_TIMELENGTH,
};
use windows::core::HRESULT;

use crate::media_type::NegotiatedType;

pub(crate) type DmoResult<T> = Result<T, HRESULT>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Done,
    NoOutput,
}

pub(crate) struct OutputMeta {
    pub(crate) time: Option<i64>,
    pub(crate) length: Option<i64>,
    pub(crate) discontinuity: bool,

    pub(crate) more: bool,
}

const DELAY_MAX: usize = 32;

fn map_err(e: PassError) -> HRESULT {
    trace!("  core error: {e:?}");
    match e {
        PassError::TypeNotSet => DMO_E_TYPE_NOT_SET,
        PassError::PictureMismatch => DMO_E_TYPE_NOT_ACCEPTED,
        PassError::NotAccepting => DMO_E_NOTACCEPTING,
        PassError::ShortInput { .. } | PassError::OutputTooSmall { .. } => E_INVALIDARG,
        PassError::CopyFailed => E_FAIL,
    }
}

pub(crate) struct Processor {
    input: Option<NegotiatedType>,
    output: Option<NegotiatedType>,
    core: Passthrough,
}

impl Processor {
    pub(crate) fn new() -> Self {
        let delay = std::env::var("PLAYBACK_IVTC_DELAY")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map_or(0, |n| n.min(DELAY_MAX));
        trace!("Processor::new delay={delay}");
        Self {
            input: None,
            output: None,
            core: Passthrough::new(delay),
        }
    }

    pub(crate) fn input_type(&self) -> DmoResult<&NegotiatedType> {
        self.input.as_ref().ok_or(DMO_E_TYPE_NOT_SET)
    }

    pub(crate) fn output_type(&self) -> DmoResult<&NegotiatedType> {
        self.output.as_ref().ok_or(DMO_E_TYPE_NOT_SET)
    }

    pub(crate) fn preferred_output_type(&self) -> DmoResult<&NegotiatedType> {
        self.output
            .as_ref()
            .or(self.input.as_ref())
            .ok_or(DMO_E_TYPE_NOT_SET)
    }

    pub(crate) fn set_input_type(&mut self, new: NegotiatedType, test_only: bool) -> DmoResult<()> {
        trace!("SetInputType test_only={test_only}: {}", new.describe());
        if test_only || self.input.as_ref() == Some(&new) {
            return Ok(());
        }
        if self.core.set_input_layout(new.info.layout) {
            trace!("  output type cleared (picture size changed)");
            self.output = None;
        }
        self.input = Some(new);
        Ok(())
    }

    pub(crate) fn set_output_type(
        &mut self,
        new: NegotiatedType,
        test_only: bool,
    ) -> DmoResult<()> {
        trace!("SetOutputType test_only={test_only}: {}", new.describe());
        if test_only {
            return self
                .core
                .check_output_layout(&new.info.layout)
                .map_err(map_err);
        }

        self.core
            .set_output_layout(new.info.layout)
            .map_err(map_err)?;
        self.output = Some(new);
        Ok(())
    }

    pub(crate) fn clear_input(&mut self) {
        self.core.clear_input();
        self.input = None;
        self.output = None;
    }

    pub(crate) fn clear_output(&mut self) {
        self.core.clear_output();
        self.output = None;
    }

    pub(crate) fn input_status_accepts_data(&self) -> DmoResult<bool> {
        self.core.accepts_input().map_err(map_err)
    }

    pub(crate) fn has_ready_output(&self) -> bool {
        self.core.has_ready()
    }

    pub(crate) fn flush(&mut self) {
        let (i, o, d) = self.core.counters();
        let dropped = self.core.flush();
        trace!(
            "flush: dropped {dropped} queued (in={i} out={o} dropped_total={})",
            d + dropped as u64
        );
    }

    pub(crate) fn discontinuity(&mut self) -> DmoResult<()> {
        trace!("discontinuity: draining {} queued", self.core.queued());
        self.core.begin_drain().map_err(map_err)
    }

    pub(crate) fn free_resources(&mut self) {
        self.core.free_resources();
    }

    pub(crate) fn process_input(
        &mut self,
        data: &[u8],
        flags: u32,
        time: i64,
        length: i64,
    ) -> DmoResult<Status> {
        let meta = SampleMeta {
            time: (flags & DMO_INPUT_DATA_BUFFERF_TIME.0 as u32 != 0).then_some(time),
            length: (flags & DMO_INPUT_DATA_BUFFERF_TIMELENGTH.0 as u32 != 0).then_some(length),
            discontinuity: flags & DMO_INPUT_DATA_BUFFERF_DISCONTINUITY.0 as u32 != 0,
        };
        let ready = self.core.push(data, meta).map_err(map_err)?;
        Ok(if ready {
            Status::Done
        } else {
            Status::NoOutput
        })
    }

    pub(crate) fn output_frame_size(&self) -> DmoResult<usize> {
        self.core.output_frame_size().map_err(map_err)
    }

    pub(crate) fn process_output(&mut self, dst: &mut [u8]) -> DmoResult<Option<OutputMeta>> {
        let out = self.core.pop_into(dst).map_err(map_err)?;
        Ok(out.map(|o| OutputMeta {
            time: o.sample.time,
            length: o.sample.length,
            discontinuity: o.sample.discontinuity,
            more: o.more,
        }))
    }

    pub(crate) fn discard_ready(&mut self) {
        self.core.discard_ready();
    }
}
