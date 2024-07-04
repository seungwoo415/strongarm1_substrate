use std::path::PathBuf;
use rust_decimal_macros::dec;
use rust_decimal::Decimal;
use sky130pdk::Sky130Pdk; 
use sky130pdk::atoll::{MosTile, TapTile};
use ucieanalog::strongarm::tb::{StrongArmTranTb, ComparatorDecision};
use ucieanalog::strongarm::{StrongArmImpl, InputKind, StrongArmParams, StrongArm}; 
use ucieanalog::tiles::{MosKind, MosTileParams, TapIo, TapTileParams, TileKind}; 
use atoll::{TileBuilder, TileWrapper}; 
use substrate::pdk::corner::Pvt;  
use sky130pdk::corner::Sky130Corner;
use substrate::context::{Context, PdkContext}; 
use ngspice::Ngspice;  
use substrate::schematic::netlist::ConvertibleNetlister;
use spice::Spice;
use spectre::Spectre;
use spice::netlist::NetlistOptions;

pub struct Sky130strongarm; 
 
impl StrongArmImpl<Sky130Pdk> for Sky130strongarm {
    type MosTile = MosTile;
    type TapTile = TapTile;
    type ViaMaker = sky130pdk::atoll::Sky130ViaMaker;

    fn mos(params: MosTileParams) -> Self::MosTile {
        MosTile::new(6, 0.15, params.mos_kind)
    }
    fn tap(params: TapTileParams) -> Self::TapTile {
        TapTile::new(2, 2)
    }
    fn via_maker() -> Self::ViaMaker {
        sky130pdk::atoll::Sky130ViaMaker
    }
    fn post_layout_hooks(cell: &mut TileBuilder<'_, Sky130Pdk>) -> substrate::error::Result<()> {
        Ok(())
    }
}

pub fn sky130_open_ctx() -> PdkContext<Sky130Pdk> {
    let pdk_root = std::env::var("SKY130_OPEN_PDK_ROOT")
        .expect("the SKY130_OPEN_PDK_ROOT environment variable must be set");
    Context::builder()
        .install(Spectre::default())
        .install(Sky130Pdk::open(pdk_root))
        .build()
        .with_pdk()
}

#[test] 
fn strongarm_sim() {
    let work_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/build/strongarm_sim");
    let input_kind = InputKind::N; 
    let pex_work_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/build/strongarm_sim/pex");
    let dut = TileWrapper::new(StrongArm::<Sky130strongarm>::new(StrongArmParams {
        nmos_kind: MosKind::Lvt,
        pmos_kind: MosKind::Lvt,
        half_tail_w: 2,
        input_pair_w: 2,
        inv_input_w: 2,
        inv_precharge_w: 2,
        precharge_w: 2,
        input_kind,
    })); 

    let pvt = Pvt {
        corner: Sky130Corner::Tt, 
        voltage: dec!(0.85), 
        temp: dec!(25.0) 
    };

    let ctx = sky130_open_ctx(); 

    for i in 0..=10 {
        for j in [
            dec!(-0.85), 
            dec!(-0.5), 
            dec!(-0.1), 
            dec!(-0.05), 
            dec!(0.05), 
            dec!(0.1), 
            dec!(0.5), 
            dec!(0.85), 
        ] {
            let vinn = dec!(0.085) * Decimal::from(i); 
            let vinp = vinn + j; 
            let work_dir = PathBuf::from(work_dir).join(format!("vinn_{vinn}_vinp_{vinp}"));

            if vinn < dec!(0) || vinp < dec!(0) || vinn > dec!(0.85) || vinp > dec!(0.85) {
                continue;
            } 

            match input_kind {
                InputKind::P => {
                    if (vinp + vinn) /dec!(2) > dec!(0.55) {
                        continue; 
                    }
                }
                InputKind::N => {
                    if (vinp + vinn) /dec!(2) > dec!(0.3) {
                        continue; 
                    }
                }
            }
            
            let tb = StrongArmTranTb::new(dut.clone(), vinp, vinn, input_kind.is_n(), pvt); 
            let decision = ctx
                .simulate(tb, work_dir)
                .expect("failed to run simulation")
                .expect("comparator output did not rail"); 
            assert_eq!(
                decision, 
                if j > dec!(0) {
                    ComparatorDecision::Pos
                } else {
                    ComparatorDecision::Neg
                }, 
                "comparator produced incorrect decision"
            )

        }
    }


}
    
fn strongarm_lvs() {
    let work_dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/build/strongarm_lvs"));
    let gds_path = work_dir.join("layout.gds"); 
    let netlist_path = work_dir.join("netlist.sp");
    let pdk_root = std::env::var("SKY130_OPEN_PDK_ROOT")
        .expect("the SKY130_OPEN_PDK_ROOT environment variable must be set");
    let ctx = Context::builder()
    .install(Spectre::default())
    .install(Sky130Pdk::open(pdk_root))
    .build()
    .with_pdk(); 
  
    let block = TileWrapper::new(StrongArm::<Sky130strongarm>::new(StrongArmParams {
        nmos_kind: MosKind::Lvt,
        pmos_kind: MosKind::Lvt,
        half_tail_w: 2,
        input_pair_w: 2,
        inv_input_w: 2,
        inv_precharge_w: 2,
        precharge_w: 2,
        input_kind: InputKind::N,
    }));

    let scir = ctx
        .export_scir(block)
        .unwrap()
        .scir
        .convert_schema::<Spice>()
        .unwrap() 
        .build()
        .unwrap();
    Spice 
        .write_scir_netlist_to_file(&scir, netlist_path, NetlistOptions::default())
        .expect("failed to write netlist");
    ctx.write_layout(block, gds_path) 
        .expect("failed to write layout");

}