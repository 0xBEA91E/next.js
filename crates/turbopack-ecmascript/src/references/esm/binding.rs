use anyhow::Result;
use swc_common::DUMMY_SP;
use swc_ecma_ast::{
    ComputedPropName, Expr, Ident, KeyValueProp, Lit, MemberExpr, MemberProp, Prop, PropName, Str,
};
use swc_ecma_visit::fields::{ExprField, PropField};
use turbopack_core::chunk::ChunkingContextVc;

use super::EsmAssetReferenceVc;
use crate::{
    chunk::EcmascriptChunkContextVc,
    code_gen::{CodeGenerateable, CodeGenerateableVc, CodeGeneration, CodeGenerationVc},
    create_visitor,
    references::{
        esm::base::{get_ident, ReferencedAsset},
        AstPathVc,
    },
};

#[turbo_tasks::value(shared)]
#[derive(Hash, Debug)]
pub struct EsmBinding {
    pub reference: EsmAssetReferenceVc,
    pub export: Option<String>,
    pub ast_path: AstPathVc,
}

#[turbo_tasks::value_impl]
impl CodeGenerateable for EsmBinding {
    #[turbo_tasks::function]
    async fn code_generation(
        self_vc: EsmBindingVc,
        _chunk_context: EcmascriptChunkContextVc,
        _context: ChunkingContextVc,
    ) -> Result<CodeGenerationVc> {
        let this = self_vc.await?;
        let mut visitors = Vec::new();
        let imported_module = this.reference.get_referenced_asset();

        fn make_expr(imported_module: Option<&str>, export: Option<&str>) -> Expr {
            if let Some(imported_module) = imported_module {
                if let Some(export) = export {
                    Expr::Member(MemberExpr {
                        span: DUMMY_SP,
                        obj: box Expr::Ident(Ident::new(imported_module.into(), DUMMY_SP)),
                        prop: MemberProp::Computed(ComputedPropName {
                            span: DUMMY_SP,
                            expr: box Expr::Lit(Lit::Str(Str {
                                span: DUMMY_SP,
                                value: export.into(),
                                raw: None,
                            })),
                        }),
                    })
                } else {
                    Expr::Ident(Ident::new(imported_module.into(), DUMMY_SP))
                }
            } else {
                Expr::Ident(Ident::new("undefined".into(), DUMMY_SP))
            }
        }

        let mut ast_path = this.ast_path.await?.clone();
        let imported_module =
            if let ReferencedAsset::Some(imported_module) = &*imported_module.await? {
                Some(get_ident(*imported_module).await?)
            } else {
                None
            };

        loop {
            match ast_path.last() {
                Some(swc_ecma_visit::AstParentKind::Expr(ExprField::Ident)) => {
                    ast_path.pop();
                    visitors.push(
                        create_visitor!(exact ast_path, visit_mut_expr(expr: &mut Expr) {
                            *expr = make_expr(imported_module.as_deref(), this.export.as_deref());
                        }),
                    );
                    break;
                }
                Some(swc_ecma_visit::AstParentKind::Prop(PropField::Shorthand)) => {
                    ast_path.pop();
                    visitors.push(
                        create_visitor!(ast_path, visit_mut_prop(prop: &mut Prop) {
                            if let Prop::Shorthand(ident) = prop {
                                *prop = Prop::KeyValue(KeyValueProp { key: PropName::Ident(ident.clone()), value: box make_expr(imported_module.as_deref(), this.export.as_deref())});
                            }
                        }),
                    );
                    break;
                }
                Some(_) => {
                    ast_path.pop();
                }
                None => break,
            }
        }

        Ok(CodeGeneration { visitors }.into())
    }
}
