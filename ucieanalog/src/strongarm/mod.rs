//! StrongARM latch layout generators.

use crate::buffer::{BufferIoSchematic, Inverter, InverterImpl, InverterParams};
use crate::tiles::{MosKind, MosTileParams, TapIo, TapTileParams, TileKind};
use atoll::route::{GreedyRouter, ViaMaker};
use atoll::{IoBuilder, Orientation, Tile, TileBuilder};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::marker::PhantomData;
use substrate::arcstr::ArcStr;
use substrate::block::Block;
use substrate::error::Result;
use substrate::geometry::align::AlignMode;
use substrate::io::{DiffPair, InOut, Input, Io, MosIo, MosIoSchematic, Output, Signal, Array, ArrayData, schematic};
use substrate::layout::ExportsLayoutData;
use substrate::pdk::Pdk;
use substrate::schematic::schema::Schema;
use substrate::schematic::ExportsNestedData;
use sky130pdk::atoll::{MosTileIo, MosTileIoSchematic};

pub mod tb;

/// The interface to a clocked differential comparator.
#[derive(Debug, Default, Clone, Io)]
pub struct ClockedDiffComparatorIo {
    /// The input differential pair.
    pub input: Input<DiffPair>,
    /// The output differential pair.
    pub output: Output<DiffPair>,
    /// The clock signal.
    pub clock: Input<Signal>,
    /// The VDD rail.
    pub vdd: InOut<Signal>,
    /// The VSS rail.
    pub vss: InOut<Signal>,
}

/// The input pair device kind of the comparator.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum InputKind {
    /// A comparator with an NMOS input pair.
    N,
    /// A comparator with a PMOS input pair.
    P,
}

impl InputKind {
    /// Returns true if the input kind is NMOS.
    pub fn is_n(&self) -> bool {
        matches!(self, InputKind::N)
    }

    /// Returns true if the input kind is PMOS.
    pub fn is_p(&self) -> bool {
        matches!(self, InputKind::P)
    }
}

/// The parameters of the [`StrongArm`] layout generator.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct StrongArmParams {
    /// The NMOS device flavor.
    pub nmos_kind: MosKind,
    /// The PMOS device flavor.
    pub pmos_kind: MosKind,
    /// The width of one half of the tail MOS device.
    pub half_tail_w: i64,
    /// The width of an input pair MOS device.
    pub input_pair_w: i64,
    /// The width of the inverter MOS devices connected to the input pair.
    pub inv_input_w: i64,
    /// The width of the inverter MOS devices connected to the precharge devices.
    pub inv_precharge_w: i64,
    /// The width of the precharge MOS devices.
    pub precharge_w: i64,
    /// The kind of the input pair MOS devices.
    pub input_kind: InputKind,
}

/// A StrongARM latch implementation.
pub trait StrongArmImpl<PDK: Pdk + Schema> {
    /// The MOS tile.
    type MosTile: Tile<PDK> + Block<Io = MosTileIo> + Clone;
    /// The tap tile.
    type TapTile: Tile<PDK> + Block<Io = TapIo> + Clone;
    /// A PDK-specific via maker.
    type ViaMaker: ViaMaker<PDK>;

    /// Creates an instance of the MOS tile.
    fn mos(params: MosTileParams) -> Self::MosTile;
    /// Creates an instance of the tap tile.
    fn tap(params: TapTileParams) -> Self::TapTile;
    /// Creates a PDK-specific via maker.
    fn via_maker() -> Self::ViaMaker;
    /// Additional layout hooks to run after the strongARM layout is complete.
    fn post_layout_hooks(_cell: &mut TileBuilder<'_, PDK>) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Io)]
struct StrongArmHalfIo {
    /// Ports that are exposed at the top level of a StrongARM.
    top_io: InOut<ClockedDiffComparatorIo>,
    /// Drains of input pair.
    input_d: InOut<DiffPair>,
    /// Drain of tail.
    tail_d: InOut<Signal>,
}

#[derive_where::derive_where(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
struct StrongArmHalf<T>(
    StrongArmParams,
    #[serde(bound(deserialize = ""))] PhantomData<fn() -> T>,
);

impl<T> StrongArmHalf<T> {
    fn new(params: StrongArmParams) -> Self {
        Self(params, PhantomData)
    }
}

impl<T: Any> Block for StrongArmHalf<T> {
    type Io = StrongArmHalfIo;

    fn id() -> ArcStr {
        substrate::arcstr::literal!("strong_arm_half")
    }

    // todo: include parameters in name
    fn name(&self) -> ArcStr {
        substrate::arcstr::literal!("strong_arm_half")
    }

    fn io(&self) -> Self::Io {
        Default::default()
    }
}

impl<T: Any> ExportsNestedData for StrongArmHalf<T> {
    type NestedData = ();
}

impl<T: Any> ExportsLayoutData for StrongArmHalf<T> {
    type LayoutData = ();
}

impl<PDK: Pdk + Schema + Sized, T: StrongArmImpl<PDK> + Any> Tile<PDK> for StrongArmHalf<T> {
    fn tile<'a>(
        &self,
        io: IoBuilder<'a, Self>,
        cell: &mut TileBuilder<'a, PDK>,
    ) -> substrate::error::Result<(
        <Self as ExportsNestedData>::NestedData,
        <Self as ExportsLayoutData>::LayoutData,
    )> {
        let (
            input_kind,
            precharge_kind,
            input_flavor,
            precharge_flavor,
            input_rail,
            precharge_rail,
        ) = match self.0.input_kind {
            InputKind::N => (
                TileKind::N,
                TileKind::P,
                self.0.nmos_kind,
                self.0.pmos_kind,
                io.schematic.top_io.vss,
                io.schematic.top_io.vdd,
            ),
            InputKind::P => (
                TileKind::P,
                TileKind::N,
                self.0.pmos_kind,
                self.0.nmos_kind,
                io.schematic.top_io.vdd,
                io.schematic.top_io.vss,
            ),
        };
        let half_tail_params = MosTileParams::new(input_flavor, input_kind, self.0.half_tail_w);
        let input_pair_params = MosTileParams::new(input_flavor, input_kind, self.0.input_pair_w);
        let inv_input_params = MosTileParams::new(input_flavor, input_kind, self.0.inv_input_w);
        let inv_precharge_params =
            MosTileParams::new(precharge_flavor, precharge_kind, self.0.inv_precharge_w);
        let precharge_params =
            MosTileParams::new(precharge_flavor, precharge_kind, self.0.precharge_w);

        let tail = io.schematic.tail_d;
        let intn = io.schematic.input_d.n;
        let intp = cell.signal("intp", Signal);
        
        let input_rail1 = cell.signal("input_rail", Array::new(2, Signal));
        let input_rail2 = cell.signal("input_rail", Array::new(1, Signal));
        let mut tail_dummy = cell.generate_connected(
            T::mos(half_tail_params),
            MosTileIoSchematic {
                sd: input_rail1,
                g: input_rail2,
                b: input_rail,
            },
        );
        cell.connect(tail_dummy.io().sd[0], input_rail);
        cell.connect(tail_dummy.io().sd[1], input_rail);
        cell.connect(tail_dummy.io().g[0], input_rail);
        
        let input_rail_tail1 = &cell.signal("input_rail_tail", Array::new(2, Signal));
        let io_stic =&cell.signal("io.schematic.top_io.clock", Array::new(1, Signal));
        let mut tail_pair = (0..2)
            .map(|_| {
                cell.generate_connected(
                    T::mos(half_tail_params),
                    MosTileIoSchematic {
                        sd: input_rail_tail1.clone(),
                        g: io_stic.clone(),
                        b: input_rail,
                    },
                )
            })
            .collect::<Vec<_>>();
        cell.connect(tail_pair[0].io().sd[0], input_rail);
        cell.connect(tail_pair[0].io().sd[1], tail);
        cell.connect(tail_pair[0].io().g[0], io.schematic.top_io.clock);
        cell.connect(tail_pair[1].io().sd[0], input_rail);
        cell.connect(tail_pair[1].io().sd[1], tail);
        cell.connect(tail_pair[1].io().g[0], io.schematic.top_io.clock);

        let mut ptap = cell.generate(T::tap(TapTileParams::new(TileKind::P, 3)));
        let ntap = cell.generate(T::tap(TapTileParams::new(TileKind::N, 3)));
        cell.connect(ptap.io().x, io.schematic.top_io.vss);
        cell.connect(ntap.io().x, io.schematic.top_io.vdd);

        let tail_int = cell.signal("tail_int", Array::new(2, Signal));
        let iosti = cell.signal("io.schematic.top_io.input", Array::new(1, Signal));
        let mut input_pair = (0..2)
            .map(|i| {
                cell.generate_connected(
                    T::mos(input_pair_params),
                    MosTileIoSchematic {
                        sd: tail_int.clone(),
                        g: iosti.clone(),
                        b: input_rail,
                    },
                )
            })
            .collect::<Vec<_>>();
        cell.connect(input_pair[0].io().sd[0], tail);
        cell.connect(input_pair[0].io().sd[1], intn);
        cell.connect(tail_pair[0].io().g[0], io.schematic.top_io.input.p);
        cell.connect(input_pair[1].io().sd[0], tail);
        cell.connect(input_pair[1].io().sd[1], intp);
        cell.connect(tail_pair[1].io().g[0], io.schematic.top_io.input.n);

        let input_rail3 = cell.signal("input_rail", Array::new(2, Signal)); 
        let input_rail4 = cell.signal("input_rail", Array::new(1, Signal));

        let mut input_dummy = cell.generate_connected(
            T::mos(input_pair_params),
            MosTileIoSchematic {
                sd: input_rail3,
                g: input_rail4,
                b: input_rail,
            },
        );
        cell.connect(input_dummy.io().sd[0], input_rail);
        cell.connect(input_dummy.io().sd[1], input_rail);
        cell.connect(tail_dummy.io().g[0], input_rail);
        
        let intniostio = cell.signal("intn_io.schematic.top_io.output", Array::new(2, Signal));
        let iostiop = cell.signal("io.schematic.top_io.output.p", Array::new(1, Signal));
        let intpiostio = cell.signal("intp_io.schematic.top_io.output", Array::new(2, Signal));
        let iostion = cell.signal("io.schematic.top_io.output.n", Array::new(1, Signal));
        let mut inv_input_pair = (0..2)
            .map(|i| {
                cell.generate_connected(
                    T::mos(inv_input_params),
                    if i == 0 {
                        MosTileIoSchematic {
                            sd: intniostio.clone(),
                            g: iostiop.clone(),
                            b: input_rail,
                        }
                    } else {
                        MosTileIoSchematic {
                            sd: intpiostio.clone(),
                            g: iostion.clone(),
                            b: input_rail,
                        }
                    },
                )
            })
            .collect::<Vec<_>>();
        cell.connect(inv_input_pair[0].io().sd[0], intn);
        cell.connect(inv_input_pair[0].io().sd[1], io.schematic.top_io.output.n);
        cell.connect(inv_input_pair[0].io().g[0], io.schematic.top_io.output.p);
        cell.connect(inv_input_pair[1].io().sd[0], intp);
        cell.connect(inv_input_pair[1].io().sd[1], io.schematic.top_io.output.p);
        cell.connect(inv_input_pair[1].io().g[0], io.schematic.top_io.output.n);

        
        let input_rail5 = cell.signal("input_rail", Array::new(2, Signal));
        let input_rail6 = cell.signal("intput_rail", Array::new(1, Signal));

        let mut inv_input_dummy = cell.generate_connected(
            T::mos(inv_input_params),
            MosTileIoSchematic {
                sd: input_rail5,
                g: input_rail6,
                b: input_rail,
            },
        );
        cell.connect(inv_input_dummy.io().sd[0], input_rail);
        cell.connect(inv_input_dummy.io().sd[1], input_rail);
        cell.connect(inv_input_dummy.io().g[0], input_rail);

        let precharge_rail_ios = cell.signal("precharge_rail_io.schematic", Array::new(2, Signal));
        let iostio1 = cell.signal("io.schematic.top_io.output", Array::new(1, Signal));
        let mut inv_precharge_pair = (0..2)
            .map(|i| {
                cell.generate_connected(
                    T::mos(inv_precharge_params),
                    MosTileIoSchematic {
                        sd: precharge_rail_ios.clone(),
                        g: iostio1.clone(),
                        b: precharge_rail,
                    },
                )
            })
            .collect::<Vec<_>>();

        cell.connect(inv_precharge_pair[0].io().sd[0], precharge_rail);
        cell.connect(inv_precharge_pair[0].io().sd[1], io.schematic.top_io.output.n);
        cell.connect(inv_precharge_pair[0].io().g[0], io.schematic.top_io.output.p);
        cell.connect(inv_precharge_pair[1].io().sd[0], precharge_rail);
        cell.connect(inv_precharge_pair[1].io().sd[1], io.schematic.top_io.output.p);
        cell.connect(inv_precharge_pair[1].io().g[0], io.schematic.top_io.output.n);
        

        let precharge_rail1 = cell.signal("precharge_rail", Array::new(2, Signal));
        let precharge_rail2 = cell.signal("precharge_rail", Array::new(1, Signal));
        let mut inv_precharge_dummy = cell.generate_connected(
            T::mos(inv_precharge_params),
            MosTileIoSchematic {
                sd: precharge_rail1,
                g: precharge_rail2, 
                b: precharge_rail,
            },
        );
        cell.connect(inv_precharge_dummy.io().sd[0], precharge_rail);
        cell.connect(inv_precharge_dummy.io().sd[1], precharge_rail);
        cell.connect(inv_precharge_dummy.io().g[0], precharge_rail);

        let precharge_rail_ios = cell.signal("precharge_rail_io.schematic", Array::new(1, Signal));
        let iost_ioc = cell.signal("io.schematic.top_io.clock", Array::new(1, Signal));

        let mut precharge_pair_a = (0..2)
            .map(|i| {
                cell.generate_connected(
                    T::mos(precharge_params),
                    MosTileIoSchematic {
                        sd: precharge_rail_ios.clone(),
                        g: iost_ioc.clone(),
                        b: precharge_rail,
                    },
                )
            })
            .collect::<Vec<_>>();
        
            cell.connect(inv_precharge_pair[0].io().sd[0], precharge_rail);
            cell.connect(inv_precharge_pair[0].io().sd[1], io.schematic.top_io.output.n);
            cell.connect(inv_precharge_pair[0].io().g[0], io.schematic.top_io.clock);
            cell.connect(inv_precharge_pair[1].io().sd[0], precharge_rail);
            cell.connect(inv_precharge_pair[1].io().sd[1],  io.schematic.top_io.output.p);
            cell.connect(inv_precharge_pair[1].io().g[0], io.schematic.top_io.clock);
        
        let precharge_rail3 = cell.signal("precharge_rail", Array::new(2, Signal)); 
        let precharge_rail4 = cell.signal("precharge_rail", Array::new(1, Signal));

        let mut precharge_pair_a_dummy = cell.generate_connected(
            T::mos(precharge_params),
            MosTileIoSchematic {
                sd: precharge_rail3,
                g: precharge_rail4,
                b: precharge_rail,
            },
        );
        cell.connect(precharge_pair_a_dummy.io().sd[0], precharge_rail);
        cell.connect(precharge_pair_a_dummy.io().sd[1], precharge_rail);
        cell.connect(precharge_pair_a_dummy.io().g[0], precharge_rail);

        let precharge_rail_int = cell.signal("precharge_rail_int", Array::new(2, Signal));
        let iostic1 = cell.signal("io.schematic.top_io.clock", Array::new(1, Signal));
        let mut precharge_pair_b = (0..2)
            .map(|i| {
                cell.generate_connected(
                    T::mos(precharge_params),
                    MosTileIoSchematic {
                        sd: precharge_rail_int.clone(),
                        g: iostic1.clone(),
                        b: precharge_rail,
                    },
                )
            })
            .collect::<Vec<_>>();
        cell.connect(precharge_pair_b[0].io().sd[0], precharge_rail);
        cell.connect(precharge_pair_b[0].io().sd[1], intn);
        cell.connect(precharge_pair_b[0].io().g[0], io.schematic.top_io.clock);
        cell.connect(precharge_pair_b[1].io().sd[0], precharge_rail);
        cell.connect(precharge_pair_b[1].io().sd[1],  intp);
        cell.connect(precharge_pair_b[1].io().g[0], io.schematic.top_io.clock);

        let precharge_rail5 = cell.signal("precharge_rail", Array::new(2, Signal));
        let precharge_rail6 = cell.signal("precharge_rail", Array::new(1, Signal));

        let mut precharge_pair_b_dummy = cell.generate_connected(
            T::mos(precharge_params),
            MosTileIoSchematic {
                sd: precharge_rail5,
                g: precharge_rail6,
                b: precharge_rail,
            },
        );
        cell.connect(precharge_pair_b_dummy.io().sd[0], precharge_rail);
        cell.connect(precharge_pair_b_dummy.io().sd[1], precharge_rail);
        cell.connect(precharge_pair_b_dummy.io().g[0], precharge_rail);

        let mut prev = ntap.lcm_bounds();

        let mut rows = [
            (&mut precharge_pair_a_dummy, &mut precharge_pair_a),
            (&mut precharge_pair_b_dummy, &mut precharge_pair_b),
            (&mut inv_precharge_dummy, &mut inv_precharge_pair),
            (&mut inv_input_dummy, &mut inv_input_pair),
            (&mut input_dummy, &mut input_pair),
            (&mut tail_dummy, &mut tail_pair),
        ];

        if self.0.input_kind == InputKind::P {
            rows.reverse();
        }

        for (dummy, mos_pair) in rows {
            dummy.align_rect_mut(prev, AlignMode::Left, 0);
            dummy.align_rect_mut(prev, AlignMode::Beneath, 0);
            prev = dummy.lcm_bounds();
            mos_pair[0].align_rect_mut(prev, AlignMode::Bottom, 0);
            mos_pair[0].align_rect_mut(prev, AlignMode::ToTheRight, 0);
            let left_rect = mos_pair[0].lcm_bounds();
            mos_pair[1].align_rect_mut(left_rect, AlignMode::Bottom, 0);
            mos_pair[1].align_rect_mut(left_rect, AlignMode::ToTheRight, 0);
        }

        ptap.align_rect_mut(prev, AlignMode::Left, 0);
        ptap.align_rect_mut(prev, AlignMode::Beneath, 0);

        let ptap = cell.draw(ptap)?;
        let ntap = cell.draw(ntap)?;
        let tail_pair = tail_pair
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _tail_dummy = cell.draw(tail_dummy)?;
        let input_pair = input_pair
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _input_dummy = cell.draw(input_dummy)?;
        let inv_nmos_pair = inv_input_pair
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _inv_nmos_dummy = cell.draw(inv_input_dummy)?;
        let _inv_pmos_pair = inv_precharge_pair
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _inv_pmos_dummy = cell.draw(inv_precharge_dummy)?;
        let _precharge_pair_a = precharge_pair_a
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _precharge_pair_a_dummy = cell.draw(precharge_pair_a_dummy)?;
        let _precharge_pair_b = precharge_pair_b
            .into_iter()
            .map(|inst| cell.draw(inst))
            .collect::<Result<Vec<_>>>()?;
        let _precharge_pair_b_dummy = cell.draw(precharge_pair_b_dummy)?;

        cell.set_top_layer(2);
        cell.set_router(GreedyRouter::new());
        cell.set_via_maker(T::via_maker());

        io.layout.top_io.vdd.set_primary(ntap.layout.io().x.primary);
        io.layout.top_io.vss.set_primary(ptap.layout.io().x.primary);
        io.layout.input_d.n.merge(input_pair[0].layout.io().sd[0].clone());
        io.layout.input_d.p.merge(input_pair[1].layout.io().sd[1].clone());
        io.layout.tail_d.merge(tail_pair[0].layout.io().sd[1].clone());
        io.layout.top_io.clock.merge(tail_pair[0].layout.io().g[0].clone());
        io.layout.top_io.input.p.merge(input_pair[0].layout.io().g[0].clone());
        io.layout.top_io.input.n.merge(input_pair[1].layout.io().g[0].clone());
        io.layout
            .top_io
            .output
            .p
            .merge(inv_nmos_pair[1].layout.io().sd[1].clone());
        io.layout
            .top_io
            .output
            .n
            .merge(inv_nmos_pair[0].layout.io().sd[1].clone());

        Ok(((), ()))
    }
}

/// A StrongARM latch.
// Layout assumes that PDK layer stack has a vertical layer 0.
#[derive_where::derive_where(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub struct StrongArm<T>(
    StrongArmParams,
    #[serde(bound(deserialize = ""))] PhantomData<fn() -> T>,
);

impl<T> StrongArm<T> {
    /// Creates a new [`StrongArm`].
    pub const fn new(params: StrongArmParams) -> Self {
        Self(params, PhantomData)
    }
}

impl<T: Any> Block for StrongArm<T> {
    type Io = ClockedDiffComparatorIo;

    fn id() -> ArcStr {
        substrate::arcstr::literal!("strong_arm")
    }

    // todo: include parameters in name
    fn name(&self) -> ArcStr {
        substrate::arcstr::literal!("strong_arm")
    }

    fn io(&self) -> Self::Io {
        Default::default()
    }
}

impl<T: Any> ExportsNestedData for StrongArm<T> {
    type NestedData = ();
}

impl<T: Any> ExportsLayoutData for StrongArm<T> {
    type LayoutData = ();
}

impl<PDK: Pdk + Schema + Sized, T: StrongArmImpl<PDK> + Any> Tile<PDK> for StrongArm<T> {
    fn tile<'a>(
        &self,
        io: IoBuilder<'a, Self>,
        cell: &mut TileBuilder<'a, PDK>,
    ) -> substrate::error::Result<(
        <Self as ExportsNestedData>::NestedData,
        <Self as ExportsLayoutData>::LayoutData,
    )> {
        let tail_d = cell.signal("tail_d", Signal::new());
        let input_d = cell.signal("input_d", DiffPair::default());

        let conn = StrongArmHalfIoSchematic {
            top_io: io.schematic.clone(),
            input_d,
            tail_d,
        };
        let left_half = cell.generate_connected(StrongArmHalf::<T>::new(self.0), conn.clone());

        let right_half = cell
            .generate_connected(StrongArmHalf::<T>::new(self.0), conn)
            .orient(Orientation::ReflectHoriz)
            .align(&left_half, AlignMode::ToTheRight, 0);

        let left_half = cell.draw(left_half)?;
        let right_half = cell.draw(right_half)?;

        cell.set_top_layer(2);
        cell.set_router(GreedyRouter::new());
        cell.set_via_maker(T::via_maker());

        io.layout.vdd.merge(left_half.layout.io().top_io.vdd);
        io.layout.vdd.merge(right_half.layout.io().top_io.vdd);
        io.layout.vss.merge(left_half.layout.io().top_io.vss);
        io.layout.vss.merge(right_half.layout.io().top_io.vss);
        io.layout.clock.merge(left_half.layout.io().top_io.clock);
        io.layout.clock.merge(right_half.layout.io().top_io.clock);
        io.layout
            .input
            .p
            .merge(left_half.layout.io().top_io.input.p);
        io.layout
            .input
            .p
            .merge(right_half.layout.io().top_io.input.p);
        io.layout
            .input
            .n
            .merge(left_half.layout.io().top_io.input.n);
        io.layout
            .input
            .n
            .merge(right_half.layout.io().top_io.input.n);
        io.layout
            .output
            .p
            .merge(left_half.layout.io().top_io.output.p);
        io.layout
            .output
            .p
            .merge(right_half.layout.io().top_io.output.p);
        io.layout
            .output
            .n
            .merge(left_half.layout.io().top_io.output.n);
        io.layout
            .output
            .n
            .merge(right_half.layout.io().top_io.output.n);

        T::post_layout_hooks(cell)?;

        Ok(((), ()))
    }
}

/// A StrongARM latch with output buffers implementation.
pub trait StrongArmWithOutputBuffersImpl<PDK: Pdk + Schema>:
    StrongArmImpl<PDK> + InverterImpl<PDK>
{
    /// The spacing between the StrongARM and the buffers in ATOLL grid coordinates.
    const BUFFER_SPACING: i64;

    /// Additional layout hooks to run after the layout is complete.
    fn post_layout_hooks(_cell: &mut TileBuilder<'_, PDK>) -> Result<()> {
        Ok(())
    }
}

/// A StrongARM latch with output buffers.
// Layout assumes that PDK layer stack has a vertical layer 0.
#[derive_where::derive_where(Copy, Clone, Debug, Hash, PartialEq, Eq)]
#[derive(Serialize, Deserialize)]
pub struct StrongArmWithOutputBuffers<T>(
    StrongArmParams,
    InverterParams,
    #[serde(bound(deserialize = ""))] PhantomData<fn() -> T>,
);

impl<T> StrongArmWithOutputBuffers<T> {
    /// Creates a new [`StrongArmWithOutputBuffers`].
    pub const fn new(sa_params: StrongArmParams, buf_params: InverterParams) -> Self {
        Self(sa_params, buf_params, PhantomData)
    }
}

impl<T: Any> Block for StrongArmWithOutputBuffers<T> {
    type Io = ClockedDiffComparatorIo;

    fn id() -> ArcStr {
        substrate::arcstr::literal!("strong_arm_with_output_buffers")
    }

    // todo: include parameters in name
    fn name(&self) -> ArcStr {
        substrate::arcstr::literal!("strong_arm_with_output_buffers")
    }

    fn io(&self) -> Self::Io {
        Default::default()
    }
}

impl<T: Any> ExportsNestedData for StrongArmWithOutputBuffers<T> {
    type NestedData = ();
}

impl<T: Any> ExportsLayoutData for StrongArmWithOutputBuffers<T> {
    type LayoutData = ();
}

impl<PDK: Pdk + Schema + Sized, T: StrongArmWithOutputBuffersImpl<PDK> + Any> Tile<PDK>
    for StrongArmWithOutputBuffers<T>
{
    fn tile<'a>(
        &self,
        io: IoBuilder<'a, Self>,
        cell: &mut TileBuilder<'a, PDK>,
    ) -> substrate::error::Result<(
        <Self as ExportsNestedData>::NestedData,
        <Self as ExportsLayoutData>::LayoutData,
    )> {
        let out = cell.signal("out", DiffPair::default());

        let strongarm = cell.generate_connected(
            StrongArm::<T>::new(self.0),
            ClockedDiffComparatorIoSchematic {
                input: io.schematic.input.clone(),
                output: out.clone(),
                clock: io.schematic.clock,
                vdd: io.schematic.vdd,
                vss: io.schematic.vss,
            },
        );

        let right_buf = cell
            .generate_connected(
                Inverter::<T>::new(self.1),
                BufferIoSchematic {
                    din: out.p,
                    dout: io.schematic.output.n,
                    vdd: io.schematic.vdd,
                    vss: io.schematic.vss,
                },
            )
            .align(&strongarm, AlignMode::CenterVertical, 0)
            .align(&strongarm, AlignMode::ToTheRight, T::BUFFER_SPACING);

        let left_buf = cell
            .generate_connected(
                Inverter::<T>::new(self.1),
                BufferIoSchematic {
                    din: out.n,
                    dout: io.schematic.output.p,
                    vdd: io.schematic.vdd,
                    vss: io.schematic.vss,
                },
            )
            .orient(Orientation::ReflectHoriz)
            .align(&strongarm, AlignMode::CenterVertical, 0)
            .align(&strongarm, AlignMode::ToTheLeft, -T::BUFFER_SPACING);

        let strongarm = cell.draw(strongarm)?;
        let right_buf = cell.draw(right_buf)?;
        let left_buf = cell.draw(left_buf)?;

        cell.set_top_layer(2);
        cell.set_router(GreedyRouter::new());
        cell.set_via_maker(<T as StrongArmImpl<PDK>>::via_maker());

        io.layout.vdd.merge(strongarm.layout.io().vdd);
        io.layout.vss.merge(strongarm.layout.io().vss);
        io.layout.clock.merge(strongarm.layout.io().clock);
        io.layout.input.p.merge(strongarm.layout.io().input.p);
        io.layout.input.n.merge(strongarm.layout.io().input.n);
        io.layout.output.p.merge(left_buf.layout.io().dout);
        io.layout.output.n.merge(right_buf.layout.io().dout);

        <T as StrongArmWithOutputBuffersImpl<PDK>>::post_layout_hooks(cell)?;

        Ok(((), ()))
    }
}